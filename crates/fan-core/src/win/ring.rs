//! Per-output lock-free state shared between the control, capture and render threads.

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering};

use rtrb::{Consumer, Producer, RingBuffer};

use crate::win::device::Format;

/// Ring depth: enough for the 200 ms offset range plus drift headroom.
const RING_SECONDS: f64 = 1.5;

pub(crate) fn new_ring(src: &Format) -> (Producer<f32>, Consumer<f32>) {
    let samples = (src.rate as f64 * RING_SECONDS) as usize * src.channels;
    RingBuffer::new(samples.max(src.channels * 1024))
}

pub(crate) struct OutputShared {
    pub(crate) device_id: String,
    /// Linear gain, f32 bits.
    pub(crate) gain: AtomicU32,
    pub(crate) muted: AtomicBool,
    /// Extra ring head-start in milliseconds, always >= 0.
    pub(crate) delay_ms: AtomicU32,
    pub(crate) connected: AtomicBool,
    pub(crate) stop: AtomicBool,
    pub(crate) underruns: AtomicU64,
    pub(crate) overruns: AtomicU64,
    pub(crate) drift_ppm: AtomicI32,
    /// Peak since the last `stats()` poll, f32 bits.
    pub(crate) peak: AtomicU32,
    /// f32 bits.
    pub(crate) latency_ms: AtomicU32,
}

impl OutputShared {
    pub(crate) fn new(device_id: String) -> Self {
        Self {
            device_id,
            gain: AtomicU32::new(1.0f32.to_bits()),
            muted: AtomicBool::new(false),
            delay_ms: AtomicU32::new(0),
            connected: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            underruns: AtomicU64::new(0),
            overruns: AtomicU64::new(0),
            drift_ppm: AtomicI32::new(0),
            peak: AtomicU32::new(0),
            latency_ms: AtomicU32::new(0),
        }
    }
}

pub(crate) fn store_f32(cell: &AtomicU32, value: f32) {
    cell.store(value.to_bits(), Ordering::Relaxed);
}

pub(crate) fn load_f32(cell: &AtomicU32) -> f32 {
    f32::from_bits(cell.load(Ordering::Relaxed))
}

pub(crate) fn raise_peak(cell: &AtomicU32, value: f32) {
    let bits = value.to_bits();
    let mut current = cell.load(Ordering::Relaxed);
    while f32::from_bits(current) < value {
        match cell.compare_exchange_weak(current, bits, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return,
            Err(seen) => current = seen,
        }
    }
}

pub(crate) fn take_peak(cell: &AtomicU32) -> f32 {
    f32::from_bits(cell.swap(0, Ordering::Relaxed))
}

pub(crate) fn db_to_linear(db: f32) -> f32 {
    if db <= -60.0 {
        0.0
    } else {
        10f32.powf(db / 20.0)
    }
}
