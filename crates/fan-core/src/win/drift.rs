//! Slow PI loop that trims the resampler ratio to hold a target ring fill.

/// Hard clamp on the ratio correction: 0.1 %.
pub(crate) const MAX_ADJUST: f64 = 0.001;

const KP: f64 = 0.02;
const KI: f64 = 0.004;
const INTEGRAL_LIMIT: f64 = MAX_ADJUST / KI;

pub(crate) struct Drift {
    integral: f64,
}

impl Drift {
    pub(crate) fn new() -> Self {
        Self { integral: 0.0 }
    }

    pub(crate) fn reset(&mut self) {
        self.integral = 0.0;
    }

    /// `error` and `target` are ring fills in source frames, `dt` in seconds.
    /// Returns the relative ratio adjustment to apply.
    pub(crate) fn update(&mut self, error: f64, target: f64, dt: f64) -> f64 {
        if target <= 0.0 {
            return 0.0;
        }
        let normalized = error / target;
        self.integral = (self.integral + normalized * dt).clamp(-INTEGRAL_LIMIT, INTEGRAL_LIMIT);
        (-(KP * normalized + KI * self.integral)).clamp(-MAX_ADJUST, MAX_ADJUST)
    }
}
