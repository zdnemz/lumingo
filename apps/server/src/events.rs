//! The event hub: one bounded broadcast channel for every WebSocket client.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use tokio::sync::broadcast;

use crate::types::{ServerEvent, StateSnapshot};

/// Capacity of the broadcast channel. A client that falls this far behind is
/// sent a fresh snapshot instead of the events it missed, so a slow browser
/// can never slow the core.
pub const EVENT_CAPACITY: usize = 64;

#[derive(Debug)]
pub struct EventHub {
    tx: broadcast::Sender<ServerEvent>,
    seq: AtomicU64,
    started: Instant,
    dev_mode: bool,
}

impl EventHub {
    pub fn new(dev_mode: bool) -> Self {
        let (tx, _) = broadcast::channel(EVENT_CAPACITY);
        Self {
            tx,
            seq: AtomicU64::new(0),
            started: Instant::now(),
            dev_mode,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ServerEvent> {
        self.tx.subscribe()
    }

    pub fn uptime_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    pub fn state(&self) -> StateSnapshot {
        StateSnapshot {
            server_version: env!("CARGO_PKG_VERSION").to_owned(),
            dev_mode: self.dev_mode,
            uptime_ms: self.uptime_ms(),
        }
    }

    /// Current sequence number. The next published event is one higher.
    pub fn current_seq(&self) -> u64 {
        self.seq.load(Ordering::SeqCst)
    }

    /// A snapshot stamped with the current sequence number.
    pub fn snapshot_event(&self) -> ServerEvent {
        ServerEvent::Snapshot {
            seq: self.current_seq(),
            state: self.state(),
        }
    }

    /// Publishes a heartbeat. Having no subscriber is normal and not an error.
    pub fn publish_heartbeat(&self) {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = self.tx.send(ServerEvent::Heartbeat {
            seq,
            uptime_ms: self.uptime_ms(),
        });
    }
}
