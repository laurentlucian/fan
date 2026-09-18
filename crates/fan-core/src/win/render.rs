//! One event-driven WASAPI render thread per output: rate + channel conversion,
//! gain, and the drift loop that keeps its ring at the target fill.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rtrb::Consumer;
use rubato::{FastFixedOut, PolynomialDegree, Resampler};
use windows::core::w;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Media::Audio::{
    IAudioRenderClient, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
};
use windows::Win32::System::Threading::{
    AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, CreateEventW,
    WaitForSingleObject,
};

use crate::types::{Error, Result};
use crate::win::device::{self, ComGuard, Format};
use crate::win::drift::Drift;
use crate::win::ring::{load_f32, raise_peak, store_f32, OutputShared};
use crate::win::EngineShared;

const DRIFT_INTERVAL: Duration = Duration::from_millis(250);
/// Ring fill to hold, in render device periods.
const TARGET_PERIODS: f64 = 2.0;

/// Ring excess over target that triggers a one-shot drop instead of waiting for
/// the PI loop. At MAX_ADJUST (0.1 %) draining 1 s of backlog would take ~17 min.
const RESYNC_MS: f64 = 50.0;

pub(crate) struct RenderParams {
    pub(crate) device_id: String,
    pub(crate) src: Format,
    pub(crate) consumer: Consumer<f32>,
    pub(crate) shared: Arc<OutputShared>,
    pub(crate) engine: Arc<EngineShared>,
}

pub(crate) fn run(params: RenderParams) {
    let shared = params.shared.clone();
    if let Err(e) = try_run(params) {
        log::warn!("render thread for {} stopped: {e}", shared.device_id);
    }
    shared.connected.store(false, Ordering::Relaxed);
}

fn try_run(params: RenderParams) -> Result<()> {
    let RenderParams {
        device_id,
        src,
        mut consumer,
        shared,
        engine,
    } = params;

    let _com = ComGuard::new()?;
    let enumerator = device::enumerator()?;

    let (client, render, event, buffer_frames, period_frames, dst) = unsafe {
        let dev = device::device_by_id(&enumerator, &device_id)?;
        let client = device::activate(&dev)?;
        let mix = device::mix_format(&client)?;
        let dst = mix.parse()?;

        let period_frames = device::period_frames(&client, &dst)?;
        let buffer_hns = (period_frames as f64 * 4.0 / dst.rate as f64 * 1e7) as i64;
        client
            .Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                buffer_hns,
                0,
                mix.as_ptr(),
                None,
            )
            .map_err(device::win_err)?;

        let event = CreateEventW(None, false, false, None).map_err(device::win_err)?;
        client.SetEventHandle(event).map_err(device::win_err)?;
        let render: IAudioRenderClient = client.GetService().map_err(device::win_err)?;
        let buffer_frames = client.GetBufferSize().map_err(device::win_err)? as usize;
        client.Start().map_err(device::win_err)?;
        (client, render, event, buffer_frames, period_frames, dst)
    };

    let mut task_index = 0u32;
    let mmcss = unsafe { AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut task_index).ok() };

    let base_ratio = dst.rate as f64 / src.rate as f64;
    let mut resampler = FastFixedOut::<f32>::new(
        base_ratio,
        1.1,
        PolynomialDegree::Cubic,
        period_frames,
        src.channels,
    )
    .map_err(|e| Error::Format(e.to_string()))?;

    let mut stage = Stage::new(&resampler, &src, &dst, buffer_frames, period_frames);
    let mut drift = Drift::new();
    let mut primed = false;
    let mut applied_delay_ms = 0u32;
    let mut last_drift = Instant::now();

    let device_latency_ms = buffer_frames as f32 / dst.rate as f32 * 1000.0;
    let wait_ms = ((period_frames as f64 / dst.rate as f64) * 2000.0)
        .ceil()
        .max(4.0) as u32;
    store_f32(&shared.latency_ms, device_latency_ms);
    shared.connected.store(true, Ordering::Relaxed);

    let result = 'run: loop {
        if shared.stop.load(Ordering::Relaxed) || engine.stop.load(Ordering::Relaxed) {
            break Ok(());
        }
        unsafe { WaitForSingleObject(event, wait_ms) };
        if shared.stop.load(Ordering::Relaxed) || engine.stop.load(Ordering::Relaxed) {
            break Ok(());
        }

        let delay_ms = shared.delay_ms.load(Ordering::Relaxed);
        if delay_ms != applied_delay_ms {
            if delay_ms > applied_delay_ms {
                // More head-start: re-prime, which renders silence until the ring refills.
                primed = false;
            } else {
                let drop_ms = (applied_delay_ms - delay_ms) as f64;
                drop_frames(&mut consumer, &mut stage, &src, drop_ms);
            }
            applied_delay_ms = delay_ms;
            drift.reset();
        }
        let target = target_frames(&src, &dst, period_frames, applied_delay_ms);

        let padding = match unsafe { client.GetCurrentPadding() } {
            Ok(p) => p as usize,
            Err(e) => break Err(device::win_err(e)),
        };
        let wanted = buffer_frames.saturating_sub(padding);
        if wanted > 0 {
            let gain = if shared.muted.load(Ordering::Relaxed) {
                0.0
            } else {
                load_f32(&shared.gain) * load_f32(&engine.master_gain)
            };
            while stage.frames() < wanted {
                let outcome = produce_chunk(
                    &mut resampler,
                    &mut stage,
                    &mut consumer,
                    &src,
                    &dst,
                    &shared,
                    gain,
                    &mut primed,
                    target,
                );
                if let Err(e) = outcome {
                    break 'run Err(e);
                }
            }
            unsafe {
                let buffer = match render.GetBuffer(wanted as u32) {
                    Ok(b) => b,
                    Err(e) => break 'run Err(device::win_err(e)),
                };
                device::f32_to_bytes(stage.take(wanted), &dst, buffer);
                if let Err(e) = render.ReleaseBuffer(wanted as u32, 0) {
                    break 'run Err(device::win_err(e));
                }
            }
        }

        let now = Instant::now();
        let dt = now.duration_since(last_drift);
        if dt >= DRIFT_INTERVAL {
            last_drift = now;
            let mut fill = (consumer.slots() / src.channels) as f64;
            let resync = RESYNC_MS * src.rate as f64 / 1000.0;
            if fill - target > resync {
                drop_samples(
                    &mut consumer,
                    &mut stage,
                    (fill - target) as usize * src.channels,
                );
                shared.overruns.fetch_add(1, Ordering::Relaxed);
                drift.reset();
                fill = (consumer.slots() / src.channels) as f64;
            }
            let adjust = drift.update(fill - target, target, dt.as_secs_f64());
            let _ = resampler.set_resample_ratio(base_ratio * (1.0 + adjust), true);
            shared
                .drift_ppm
                .store((adjust * 1e6) as i32, Ordering::Relaxed);
            store_f32(
                &shared.latency_ms,
                device_latency_ms + (fill as f32 / src.rate as f32) * 1000.0,
            );
        }
    };

    unsafe {
        let _ = client.Stop();
        if let Some(mmcss) = mmcss {
            let _ = AvRevertMmThreadCharacteristics(mmcss);
        }
        let _ = CloseHandle(event);
    }
    result
}

fn target_frames(src: &Format, dst: &Format, period_frames: usize, delay_ms: u32) -> f64 {
    let periods = TARGET_PERIODS * period_frames as f64 * src.rate as f64 / dst.rate as f64;
    periods + delay_ms as f64 * src.rate as f64 / 1000.0
}

/// Shrinking the offset is a one-shot step: discard that much of the ring.
fn drop_frames(consumer: &mut Consumer<f32>, stage: &mut Stage, src: &Format, ms: f64) {
    drop_samples(
        consumer,
        stage,
        (ms * src.rate as f64 / 1000.0) as usize * src.channels,
    )
}

/// Discard `samples` interleaved samples from the ring.
fn drop_samples(consumer: &mut Consumer<f32>, stage: &mut Stage, mut remaining: usize) {
    while remaining > 0 {
        let chunk = remaining.min(stage.scratch.len());
        let (popped, _) = consumer.pop_partial_slice(&mut stage.scratch[..chunk]);
        if popped.is_empty() {
            return;
        }
        remaining -= popped.len();
    }
}

#[allow(clippy::too_many_arguments)]
fn produce_chunk(
    resampler: &mut FastFixedOut<f32>,
    stage: &mut Stage,
    consumer: &mut Consumer<f32>,
    src: &Format,
    dst: &Format,
    shared: &OutputShared,
    gain: f32,
    primed: &mut bool,
    target: f64,
) -> Result<()> {
    stage.compact();

    let need = resampler.input_frames_next();
    let wanted = need * src.channels;

    if !*primed {
        let fill = (consumer.slots() / src.channels) as f64;
        if fill >= target {
            // Start at exactly target: a backlog inherited here is permanent latency.
            if fill > target {
                drop_samples(consumer, stage, (fill - target) as usize * src.channels);
            }
            *primed = true;
        }
    }

    let Stage {
        scratch,
        planar_in,
        planar_out,
        data,
        len,
        ..
    } = stage;

    let got = if *primed {
        let (popped, _) = consumer.pop_partial_slice(&mut scratch[..wanted]);
        popped.len()
    } else {
        0
    };
    if got < wanted {
        scratch[got..wanted].fill(0.0);
        if *primed {
            shared.underruns.fetch_add(1, Ordering::Relaxed);
            if got == 0 {
                *primed = false;
            }
        }
    }

    for frame in 0..need {
        for (ch, plane) in planar_in.iter_mut().enumerate() {
            plane[frame] = scratch[frame * src.channels + ch];
        }
    }

    let (_, produced) = resampler
        .process_into_buffer(&planar_in[..], &mut planar_out[..], None)
        .map_err(|e| Error::Format(e.to_string()))?;

    let produced = produced.min((data.len() - *len) / dst.channels);
    let mut peak = 0.0f32;
    for frame in 0..produced {
        for ch in 0..dst.channels {
            let value = map_channel(planar_out, frame, ch, src.channels, dst.channels) * gain;
            peak = peak.max(value.abs());
            data[*len + ch] = value;
        }
        *len += dst.channels;
    }
    raise_peak(&shared.peak, peak);
    Ok(())
}

fn map_channel(
    planes: &[Vec<f32>],
    frame: usize,
    ch: usize,
    src_channels: usize,
    dst_channels: usize,
) -> f32 {
    if src_channels == 1 {
        planes[0][frame]
    } else if dst_channels == 1 {
        planes[..src_channels].iter().map(|p| p[frame]).sum::<f32>() / src_channels as f32
    } else if ch < src_channels {
        planes[ch][frame]
    } else {
        0.0
    }
}

/// Preallocated working set for the render thread: nothing here allocates once built.
struct Stage {
    scratch: Vec<f32>,
    planar_in: Vec<Vec<f32>>,
    planar_out: Vec<Vec<f32>>,
    data: Vec<f32>,
    head: usize,
    len: usize,
    dst_channels: usize,
}

impl Stage {
    fn new(
        resampler: &FastFixedOut<f32>,
        src: &Format,
        dst: &Format,
        buffer_frames: usize,
        period_frames: usize,
    ) -> Self {
        let capacity = (buffer_frames + period_frames * 4) * dst.channels;
        Self {
            scratch: vec![0.0; resampler.input_frames_max() * src.channels],
            planar_in: resampler.input_buffer_allocate(true),
            planar_out: resampler.output_buffer_allocate(true),
            data: vec![0.0; capacity],
            head: 0,
            len: 0,
            dst_channels: dst.channels,
        }
    }

    fn frames(&self) -> usize {
        (self.len - self.head) / self.dst_channels
    }

    /// Returns `frames` of interleaved output and advances the read cursor.
    fn take(&mut self, frames: usize) -> &[f32] {
        let samples = frames * self.dst_channels;
        let start = self.head;
        self.head += samples;
        &self.data[start..start + samples]
    }

    /// Slide unread data to the front so a new chunk always fits.
    fn compact(&mut self) {
        if self.head > 0 {
            self.data.copy_within(self.head..self.len, 0);
            self.len -= self.head;
            self.head = 0;
        }
    }
}
