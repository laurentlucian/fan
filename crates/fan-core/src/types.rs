use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub kind: DeviceKind,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DeviceKind {
    Speakers,
    Headphones,
    Bluetooth,
    Hdmi,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputConfig {
    pub device_id: String,
    pub offset_ms: i32,
    pub gain_db: f32,
    pub muted: bool,
}

impl OutputConfig {
    pub fn new(device_id: impl Into<String>) -> Self {
        Self {
            device_id: device_id.into(),
            offset_ms: 0,
            gain_db: 0.0,
            muted: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EngineConfig {
    /// None = follow the Windows default render endpoint.
    pub source_device_id: Option<String>,
    pub outputs: Vec<OutputConfig>,
    pub master_gain_db: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputStats {
    pub device_id: String,
    pub connected: bool,
    pub latency_ms: f32,
    pub underruns: u64,
    pub overruns: u64,
    /// Resampler ratio deviation from 1.0, as reported by the drift loop.
    pub drift_ppm: f32,
    /// Peak level 0.0..=1.0 since the last poll.
    pub peak: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineStats {
    pub running: bool,
    pub source_name: String,
    pub source_peak: f32,
    pub outputs: Vec<OutputStats>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unsupported platform: FAN requires Windows")]
    Unsupported,
    #[error("no default render device")]
    NoDefaultDevice,
    #[error("device not found: {0}")]
    DeviceNotFound(String),
    #[error("engine already running")]
    AlreadyRunning,
    #[error("audio format unsupported: {0}")]
    Format(String),
    #[error("windows error: {0}")]
    Windows(String),
}

pub type Result<T> = std::result::Result<T, Error>;
