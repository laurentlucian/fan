//! Loopback capture of the source render endpoint, fanned into one ring per output.

use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Arc;
use std::time::Instant;

use rtrb::Producer;
use windows::core::w;
use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    IAudioCaptureClient, AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_LOOPBACK,
};
use windows::Win32::System::Threading::{
    AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, CreateEventW,
    WaitForSingleObject,
};

use crate::types::Result;
use crate::win::device::{self, ComGuard, Format};
use crate::win::ring::{raise_peak, OutputShared};
use crate::win::EngineShared;

pub(crate) struct Sink {
    pub(crate) shared: Arc<OutputShared>,
    pub(crate) producer: Producer<f32>,
}

pub(crate) enum CaptureCmd {
    Add(Sink),
    Remove(String),
}

pub(crate) struct CaptureParams {
    pub(crate) device_id: String,
    pub(crate) format: Format,
    pub(crate) engine: Arc<EngineShared>,
    pub(crate) cmds: Receiver<CaptureCmd>,
    pub(crate) sinks: Vec<Sink>,
}

pub(crate) fn run(params: CaptureParams) {
    if let Err(e) = try_run(params) {
        log::warn!("capture thread stopped: {e}");
    }
}

fn try_run(params: CaptureParams) -> Result<()> {
    let CaptureParams {
        device_id,
        format,
        engine,
        cmds,
        mut sinks,
    } = params;

    let _com = ComGuard::new()?;
    let enumerator = device::enumerator()?;

    let (client, capture, event, buffer_frames, period_frames) = unsafe {
        let dev = device::device_by_id(&enumerator, &device_id)?;
        let client = device::activate(&dev)?;
        let mix = device::mix_format(&client)?;
        let actual = mix.parse()?;

        let period_frames = device::period_frames(&client, &actual)?;
        let buffer_hns = (period_frames as f64 * 4.0 / actual.rate as f64 * 1e7) as i64;
        client
            .Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                buffer_hns,
                0,
                mix.as_ptr(),
                None,
            )
            .map_err(device::win_err)?;

        let event = CreateEventW(None, false, false, None).map_err(device::win_err)?;
        client.SetEventHandle(event).map_err(device::win_err)?;
        let capture: IAudioCaptureClient = client.GetService().map_err(device::win_err)?;
        let buffer_frames = client.GetBufferSize().map_err(device::win_err)? as usize;
        client.Start().map_err(device::win_err)?;
        (client, capture, event, buffer_frames, period_frames)
    };

    let mut task_index = 0u32;
    let mmcss = unsafe { AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut task_index).ok() };

    let channels = format.channels;
    let mut scratch = vec![0.0f32; buffer_frames * channels];
    let silence = vec![0.0f32; buffer_frames * channels];
    let wait_ms = ((period_frames as f64 / format.rate as f64) * 1000.0)
        .ceil()
        .max(2.0) as u32;
    // Frames handed to the rings vs. frames real time has called for. Tracking both
    // is what stops a timed-out wait from injecting silence that a late packet then
    // also covers -- that double count is permanent ring backlog, i.e. latency.
    let started = Instant::now();
    let mut delivered: u64 = 0;

    while !engine.stop.load(Ordering::Relaxed) {
        drain_commands(&cmds, &mut sinks);

        let signalled = unsafe { WaitForSingleObject(event, wait_ms) } == WAIT_OBJECT_0;
        if engine.stop.load(Ordering::Relaxed) {
            break;
        }

        let mut emitted = false;
        if signalled {
            loop {
                let next = match unsafe { capture.GetNextPacketSize() } {
                    Ok(n) => n as usize,
                    Err(e) => return Err(device::win_err(e)),
                };
                if next == 0 {
                    break;
                }
                let frames = read_packet(&capture, &format, &mut scratch)?;
                if frames == 0 {
                    continue;
                }
                let data = &scratch[..frames * channels];
                let mut peak = 0.0f32;
                for v in data {
                    peak = peak.max(v.abs());
                }
                raise_peak(&engine.source_peak, peak);
                distribute(&mut sinks, data);
                delivered += frames as u64;
                emitted = true;
            }
        }

        let expected = (started.elapsed().as_secs_f64() * format.rate as f64) as u64;
        if emitted {
            // Audio is flowing: never carry a wall-clock deficit against the device clock.
            delivered = delivered.max(expected);
        } else {
            // Loopback goes silent without events; top up only the real deficit.
            let deficit = expected.saturating_sub(delivered) as usize;
            if deficit >= period_frames {
                let frames = deficit.min(buffer_frames);
                distribute(&mut sinks, &silence[..frames * channels]);
                delivered += frames as u64;
            }
        }
    }

    unsafe {
        let _ = client.Stop();
        if let Some(mmcss) = mmcss {
            let _ = AvRevertMmThreadCharacteristics(mmcss);
        }
        let _ = CloseHandle(event);
    }
    Ok(())
}

fn read_packet(
    capture: &IAudioCaptureClient,
    format: &Format,
    scratch: &mut [f32],
) -> Result<usize> {
    unsafe {
        let mut data = std::ptr::null_mut();
        let mut frames = 0u32;
        let mut flags = 0u32;
        capture
            .GetBuffer(&mut data, &mut frames, &mut flags, None, None)
            .map_err(device::win_err)?;

        let frames = (frames as usize).min(scratch.len() / format.channels);
        let samples = frames * format.channels;
        if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
            scratch[..samples].fill(0.0);
        } else {
            device::bytes_to_f32(data, format, &mut scratch[..samples]);
        }
        capture
            .ReleaseBuffer(frames as u32)
            .map_err(device::win_err)?;
        Ok(frames)
    }
}

fn distribute(sinks: &mut [Sink], data: &[f32]) {
    for sink in sinks.iter_mut() {
        let (_, dropped) = sink.producer.push_partial_slice(data);
        if !dropped.is_empty() {
            sink.shared.overruns.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn drain_commands(cmds: &Receiver<CaptureCmd>, sinks: &mut Vec<Sink>) {
    loop {
        match cmds.try_recv() {
            Ok(CaptureCmd::Add(sink)) => {
                sinks.retain(|s| s.shared.device_id != sink.shared.device_id);
                sinks.push(sink);
            }
            Ok(CaptureCmd::Remove(id)) => sinks.retain(|s| s.shared.device_id != id),
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => return,
        }
    }
}
