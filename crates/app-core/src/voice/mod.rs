//! The voice loop: listen, transcribe, think, speak.
//!
//! The orchestration lives here, not in a binary, so `tools/tutor-cli` and later
//! `apps/server` (through [`crate::SessionService`]) run the same code.

mod clock;
mod latency;

pub use clock::{LoopClock, SystemLoopClock};
pub use latency::{
    LatencyParts, LatencyRecord, LatencySummary, Quantiles, Stamps, TurnLatency, duration_ms,
    percentile,
};
