//! Elapsed time owned by one server runtime. Wire and persisted timestamps
//! continue to use the wall clock; local expiry can never move backwards.
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

#[derive(Debug)]
pub(crate) struct ServerLocalClock {
    started_at: Instant,
    base_seconds: f64,
    override_seconds: Option<f64>,
    high_water: AtomicU64,
    pub(crate) independent_override: bool,
}

impl Default for ServerLocalClock {
    fn default() -> Self {
        Self {
            started_at: Instant::now(),
            base_seconds: 0.0,
            override_seconds: None,
            high_water: AtomicU64::new(0),
            independent_override: false,
        }
    }
}

impl ServerLocalClock {
    pub(crate) fn now(&self) -> f64 {
        let sampled = self
            .override_seconds
            .unwrap_or_else(|| self.base_seconds + self.started_at.elapsed().as_secs_f64());
        self.observe(sampled)
    }

    pub(crate) fn observe(&self, sampled: f64) -> f64 {
        // Nonnegative finite IEEE doubles have the same integer/float order.
        let bits = if sampled.is_finite() && sampled > 0.0 {
            sampled.to_bits()
        } else {
            0
        };
        f64::from_bits(self.high_water.fetch_max(bits, Ordering::Relaxed).max(bits))
    }

    pub(crate) fn set_override(&mut self, seconds: Option<f64>) {
        let seconds = seconds.filter(|value| value.is_finite() && *value >= 0.0);
        if let (None, Some(_)) = (seconds, self.override_seconds) {
            self.base_seconds = self.now();
            self.started_at = Instant::now();
        }
        self.override_seconds = seconds;
        let _ = self.now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_clock_never_rewinds_or_accepts_nonfinite_samples() {
        let clock = ServerLocalClock::default();
        assert_eq!(clock.observe(-0.0).to_bits(), 0.0_f64.to_bits());
        assert_eq!(clock.observe(100.0), 100.0);
        for value in [-0.0, 50.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(clock.observe(value), 100.0);
        }
        assert_eq!(clock.observe(101.0), 101.0);
    }

    #[test]
    fn local_clock_leaving_override_reanchors_elapsed_time() {
        let mut clock = ServerLocalClock::default();
        clock.set_override(Some(1000.0));
        clock.set_override(None);
        assert!(clock.now() >= 1000.0);
        clock.started_at -= std::time::Duration::from_secs(2);
        assert!(clock.now() >= 1002.0);
    }

    #[test]
    fn invalid_override_resumes_elapsed_time_without_poisoning_or_freezing_it() {
        for invalid in [-1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut clock = ServerLocalClock::default();
            clock.set_override(Some(1000.0));
            clock.set_override(Some(invalid));
            assert!(clock.now() >= 1000.0);
            clock.started_at -= std::time::Duration::from_secs(2);
            assert!(
                clock.now() >= 1002.0,
                "invalid override must resume elapsed sampling"
            );
        }
    }
}
