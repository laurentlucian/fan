//! Non-Windows build: compiles so the workspace checks on macOS/Linux, does nothing.
use crate::types::*;

pub fn list_outputs() -> Result<Vec<DeviceInfo>> {
    Err(Error::Unsupported)
}

pub struct Engine;

impl Engine {
    pub fn start(_config: EngineConfig) -> Result<Self> {
        Err(Error::Unsupported)
    }
    pub fn apply(&self, _config: EngineConfig) -> Result<()> {
        Err(Error::Unsupported)
    }
    pub fn stats(&self) -> EngineStats {
        EngineStats {
            running: false,
            source_name: String::new(),
            source_peak: 0.0,
            outputs: Vec::new(),
        }
    }
    pub fn stop(self) {}
}
