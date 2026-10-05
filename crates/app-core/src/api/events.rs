//! The WebSocket event stream.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::state::StateSnapshot;

/// One message on the event stream. Every event carries a sequence number that
/// grows by one per published event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
#[ts(export)]
pub enum ServerEvent {
    /// Always the first message on a connection, and sent again to a client that
    /// fell behind. A client replaces its state with it.
    Snapshot { seq: u64, state: StateSnapshot },
    /// Periodic proof that the stream is alive.
    Heartbeat { seq: u64, uptime_ms: u64 },
}

impl ServerEvent {
    /// The sequence number of the event.
    pub fn seq(&self) -> u64 {
        match self {
            Self::Snapshot { seq, .. } | Self::Heartbeat { seq, .. } => *seq,
        }
    }
}
