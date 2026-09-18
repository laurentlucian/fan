//! Fan one Windows audio stream out to many render endpoints.
//!
//! Source = a render endpoint captured in WASAPI loopback mode. Its mix is
//! pushed into one lock-free ring per target device, each drained by its own
//! event-driven render thread with independent drift correction.

pub mod types;
pub use types::*;

#[cfg(windows)]
mod win;

#[cfg(windows)]
pub use win::{list_outputs, Engine};

#[cfg(not(windows))]
mod stub;

#[cfg(not(windows))]
pub use stub::{list_outputs, Engine};
