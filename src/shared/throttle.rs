//! Minimal cooldown helper.
//!
//! [`Throttle::check`] returns `true` if the configured interval has
//! elapsed since the last successful check, recording the new timestamp.
//! Otherwise it returns `false` and leaves the timer alone.

use std::time::{Duration, Instant};

pub struct Throttle {
    last: Option<Instant>,
    interval: Duration,
}

impl Throttle {
    /// Ready on the first call — `check()` succeeds immediately, then enforces
    /// `interval` between subsequent hits.
    pub fn ready(interval: Duration) -> Self {
        Self {
            last: None,
            interval,
        }
    }

    /// Starts in a cooling-down state — the first `check()` only succeeds
    /// once `interval` has elapsed since construction.
    pub fn with_cooldown(interval: Duration) -> Self {
        Self {
            last: Some(Instant::now()),
            interval,
        }
    }

    pub fn check(&mut self) -> bool {
        let ready = self.last.is_none_or(|t| t.elapsed() >= self.interval);
        if ready {
            self.last = Some(Instant::now());
        }
        ready
    }
}
