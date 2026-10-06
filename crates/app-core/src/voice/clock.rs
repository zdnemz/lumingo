//! The clock the voice loop measures latency with.
//!
//! `crate::Clock` gives calendar time for stored rows. Latency needs a steady
//! clock that never jumps back, and tests need one they can move by hand, so the
//! loop reads this trait and nothing else. Every latency stamp in a turn comes
//! from one instance, which is what makes the parts add up exactly.

use std::time::{Duration, Instant};

/// A monotonic clock. Readings are offsets from an origin the clock chooses, so
/// only differences mean anything.
pub trait LoopClock: Send + Sync {
    fn now(&self) -> Duration;
}

/// The real clock, over `Instant`.
#[derive(Debug, Clone, Copy)]
pub struct SystemLoopClock {
    origin: Instant,
}

impl SystemLoopClock {
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Default for SystemLoopClock {
    fn default() -> Self {
        Self::new()
    }
}

impl LoopClock for SystemLoopClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_clock_never_goes_back() {
        let clock = SystemLoopClock::new();
        let mut last = clock.now();
        for _ in 0..1000 {
            let next = clock.now();
            assert!(next >= last);
            last = next;
        }
    }
}
