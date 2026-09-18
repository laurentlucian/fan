//! MMDevice enumeration and WASAPI format parsing.

use windows::core::HSTRING;
use windows::Win32::Devices::FunctionDiscovery::{
    PKEY_DeviceInterface_FriendlyName, PKEY_Device_FriendlyName,
};
use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::Media::Audio::{
    eConsole, eRender, DigitalAudioDisplayDevice as FF_HDMI, Handset as FF_HANDSET,
    Headphones as FF_HEADPHONES, Headset as FF_HEADSET, IAudioClient, IMMDevice,
    IMMDeviceEnumerator, LineLevel as FF_LINE, MMDeviceEnumerator, PKEY_AudioEndpoint_FormFactor,
    Speakers as FF_SPEAKERS, DEVICE_STATE_ACTIVE, WAVEFORMATEX, WAVEFORMATEXTENSIBLE,
    WAVE_FORMAT_PCM,
};
use windows::Win32::Media::KernelStreaming::{KSDATAFORMAT_SUBTYPE_PCM, WAVE_FORMAT_EXTENSIBLE};
use windows::Win32::Media::Multimedia::{KSDATAFORMAT_SUBTYPE_IEEE_FLOAT, WAVE_FORMAT_IEEE_FLOAT};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_MULTITHREADED, STGM_READ,
};
use windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY;

use crate::types::{DeviceInfo, DeviceKind, Error, Result};

pub(crate) fn win_err(e: windows::core::Error) -> Error {
    Error::Windows(e.to_string())
}

/// Per-thread COM apartment. Every thread touching WASAPI needs one alive.
pub(crate) struct ComGuard {
    owned: bool,
}

impl ComGuard {
    pub(crate) fn new() -> Result<Self> {
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if hr == RPC_E_CHANGED_MODE {
            return Ok(Self { owned: false });
        }
        if hr.is_err() {
            return Err(Error::Windows(format!("CoInitializeEx: {hr:?}")));
        }
        Ok(Self { owned: true })
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.owned {
            unsafe { CoUninitialize() };
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SampleFormat {
    F32,
    I16,
    I32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Format {
    pub(crate) rate: u32,
    pub(crate) channels: usize,
    pub(crate) sample: SampleFormat,
    pub(crate) frame_bytes: usize,
}

/// Owns the `WAVEFORMATEX` returned by `GetMixFormat` (CoTaskMem allocated).
pub(crate) struct MixFormat(*mut WAVEFORMATEX);

impl MixFormat {
    pub(crate) fn as_ptr(&self) -> *const WAVEFORMATEX {
        self.0
    }

    pub(crate) fn parse(&self) -> Result<Format> {
        unsafe { parse_waveformat(self.0) }
    }
}

impl Drop for MixFormat {
    fn drop(&mut self) {
        unsafe { CoTaskMemFree(Some(self.0 as *const _)) };
    }
}

unsafe fn parse_waveformat(p: *const WAVEFORMATEX) -> Result<Format> {
    let tag = (*p).wFormatTag as u32;
    let bits = (*p).wBitsPerSample;
    let channels = (*p).nChannels as usize;
    let rate = (*p).nSamplesPerSec;
    let frame_bytes = (*p).nBlockAlign as usize;

    let sample = if tag == WAVE_FORMAT_EXTENSIBLE {
        let sub = (*(p as *const WAVEFORMATEXTENSIBLE)).SubFormat;
        match (sub, bits) {
            (KSDATAFORMAT_SUBTYPE_IEEE_FLOAT, 32) => SampleFormat::F32,
            (KSDATAFORMAT_SUBTYPE_PCM, 16) => SampleFormat::I16,
            (KSDATAFORMAT_SUBTYPE_PCM, 32) => SampleFormat::I32,
            _ => return Err(Error::Format(format!("extensible subformat, {bits} bits"))),
        }
    } else {
        match (tag, bits) {
            (WAVE_FORMAT_IEEE_FLOAT, 32) => SampleFormat::F32,
            (WAVE_FORMAT_PCM, 16) => SampleFormat::I16,
            (WAVE_FORMAT_PCM, 32) => SampleFormat::I32,
            _ => return Err(Error::Format(format!("tag {tag}, {bits} bits"))),
        }
    };

    // The raw copies below index by channels * sample size, so padded frames would overrun.
    if channels == 0 || rate == 0 || frame_bytes != channels * (bits as usize / 8) {
        return Err(Error::Format(format!(
            "unsupported frame layout: {channels} ch, {bits} bits, {frame_bytes} bytes"
        )));
    }
    Ok(Format {
        rate,
        channels,
        sample,
        frame_bytes,
    })
}

pub(crate) fn enumerator() -> Result<IMMDeviceEnumerator> {
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(win_err) }
}

pub(crate) unsafe fn device_id(dev: &IMMDevice) -> Result<String> {
    let raw = dev.GetId().map_err(win_err)?;
    let id = raw.to_string().unwrap_or_default();
    CoTaskMemFree(Some(raw.0 as *const _));
    Ok(id)
}

unsafe fn prop_string(dev: &IMMDevice, key: *const PROPERTYKEY) -> Option<String> {
    let store = dev.OpenPropertyStore(STGM_READ).ok()?;
    let value = store.GetValue(key).ok()?;
    let text = value.to_string();
    (!text.is_empty()).then_some(text)
}

unsafe fn prop_u32(dev: &IMMDevice, key: *const PROPERTYKEY) -> Option<u32> {
    let store = dev.OpenPropertyStore(STGM_READ).ok()?;
    let value = store.GetValue(key).ok()?;
    u32::try_from(&value).ok()
}

/// Best effort: the form factor has no Bluetooth value, so fall back to name sniffing.
unsafe fn device_kind(dev: &IMMDevice, name: &str) -> DeviceKind {
    let interface = prop_string(dev, &PKEY_DeviceInterface_FriendlyName).unwrap_or_default();
    if name.to_lowercase().contains("bluetooth") || interface.to_lowercase().contains("bluetooth") {
        return DeviceKind::Bluetooth;
    }
    let Some(ff) = prop_u32(dev, &PKEY_AudioEndpoint_FormFactor) else {
        return DeviceKind::Other;
    };
    let ff = ff as i32;
    if ff == FF_SPEAKERS.0 || ff == FF_LINE.0 {
        DeviceKind::Speakers
    } else if ff == FF_HEADPHONES.0 || ff == FF_HEADSET.0 || ff == FF_HANDSET.0 {
        DeviceKind::Headphones
    } else if ff == FF_HDMI.0 {
        DeviceKind::Hdmi
    } else {
        DeviceKind::Other
    }
}

pub(crate) unsafe fn enumerate(enumerator: &IMMDeviceEnumerator) -> Result<Vec<DeviceInfo>> {
    let default_id = default_render_id(enumerator).unwrap_or_default();
    let collection = enumerator
        .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
        .map_err(win_err)?;
    let count = collection.GetCount().map_err(win_err)?;
    let mut devices = Vec::with_capacity(count as usize);
    for i in 0..count {
        let Ok(dev) = collection.Item(i) else {
            continue;
        };
        let Ok(id) = device_id(&dev) else { continue };
        let name = prop_string(&dev, &PKEY_Device_FriendlyName).unwrap_or_else(|| id.clone());
        let kind = device_kind(&dev, &name);
        devices.push(DeviceInfo {
            is_default: id == default_id,
            id,
            name,
            kind,
        });
    }
    Ok(devices)
}

pub(crate) unsafe fn default_render_id(enumerator: &IMMDeviceEnumerator) -> Result<String> {
    let dev = enumerator
        .GetDefaultAudioEndpoint(eRender, eConsole)
        .map_err(|_| Error::NoDefaultDevice)?;
    device_id(&dev)
}

pub(crate) unsafe fn device_by_id(enumerator: &IMMDeviceEnumerator, id: &str) -> Result<IMMDevice> {
    enumerator
        .GetDevice(&HSTRING::from(id))
        .map_err(|_| Error::DeviceNotFound(id.to_string()))
}

pub(crate) unsafe fn is_active(enumerator: &IMMDeviceEnumerator, id: &str) -> bool {
    device_by_id(enumerator, id)
        .ok()
        .and_then(|d| d.GetState().ok())
        .is_some_and(|state| state == DEVICE_STATE_ACTIVE)
}

pub(crate) unsafe fn device_name(enumerator: &IMMDeviceEnumerator, id: &str) -> String {
    device_by_id(enumerator, id)
        .ok()
        .and_then(|d| prop_string(&d, &PKEY_Device_FriendlyName))
        .unwrap_or_else(|| id.to_string())
}

pub(crate) unsafe fn activate(dev: &IMMDevice) -> Result<IAudioClient> {
    dev.Activate::<IAudioClient>(CLSCTX_ALL, None)
        .map_err(win_err)
}

pub(crate) unsafe fn mix_format(client: &IAudioClient) -> Result<MixFormat> {
    Ok(MixFormat(client.GetMixFormat().map_err(win_err)?))
}

/// Default engine period, in frames of `format`.
pub(crate) unsafe fn period_frames(client: &IAudioClient, format: &Format) -> Result<usize> {
    let mut default_hns = 0i64;
    client
        .GetDevicePeriod(Some(&mut default_hns), None)
        .map_err(win_err)?;
    let frames = (default_hns as f64 * 1e-7 * format.rate as f64).round() as usize;
    Ok(frames.max(32))
}

/// Interleaved device bytes -> interleaved f32.
pub(crate) unsafe fn bytes_to_f32(src: *const u8, format: &Format, out: &mut [f32]) {
    match format.sample {
        SampleFormat::F32 => {
            std::ptr::copy_nonoverlapping(src as *const f32, out.as_mut_ptr(), out.len())
        }
        SampleFormat::I16 => {
            let p = src as *const i16;
            for (i, v) in out.iter_mut().enumerate() {
                *v = *p.add(i) as f32 / 32768.0;
            }
        }
        SampleFormat::I32 => {
            let p = src as *const i32;
            for (i, v) in out.iter_mut().enumerate() {
                *v = *p.add(i) as f32 / 2147483648.0;
            }
        }
    }
}

/// Interleaved f32 -> interleaved device bytes.
pub(crate) unsafe fn f32_to_bytes(src: &[f32], format: &Format, dst: *mut u8) {
    match format.sample {
        SampleFormat::F32 => {
            std::ptr::copy_nonoverlapping(src.as_ptr(), dst as *mut f32, src.len())
        }
        SampleFormat::I16 => {
            let p = dst as *mut i16;
            for (i, v) in src.iter().enumerate() {
                *p.add(i) = (v.clamp(-1.0, 1.0) * 32767.0) as i16;
            }
        }
        SampleFormat::I32 => {
            let p = dst as *mut i32;
            for (i, v) in src.iter().enumerate() {
                *p.add(i) = (v.clamp(-1.0, 1.0) * 2147483520.0) as i32;
            }
        }
    }
}
