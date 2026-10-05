//! Types that cross the HTTP and WebSocket API. They are defined here once and
//! exported to `apps/web/src/generated/` by ts-rs when the tests run.

use serde::Serialize;
use ts_rs::TS;

/// Full picture of the server at one moment. Returned by `GET /api/state` and
/// sent as the first WebSocket message.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct StateSnapshot {
    pub server_version: String,
    pub dev_mode: bool,
    pub uptime_ms: u64,
}

/// One message on the event stream. Every event carries a sequence number.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(tag = "type")]
#[ts(export)]
pub enum ServerEvent {
    /// Always the first message on a connection. A client replaces its state with it.
    Snapshot { seq: u64, state: StateSnapshot },
    /// Periodic proof that the stream is alive.
    Heartbeat { seq: u64, uptime_ms: u64 },
}

impl ServerEvent {
    pub fn seq(&self) -> u64 {
        match self {
            Self::Snapshot { seq, .. } | Self::Heartbeat { seq, .. } => *seq,
        }
    }
}
