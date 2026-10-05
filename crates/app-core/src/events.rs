//! The event bus: one bounded broadcast channel for every WebSocket client.
//!
//! Capacity is [`EVENT_CAPACITY`] events. Publishing never waits and never
//! fails because nobody listens. When a subscriber falls more than the capacity
//! behind, the channel drops the oldest events for that subscriber only and its
//! next receive reports `Lagged`; the transport then sends it a fresh
//! `Snapshot` instead of the missed events. A slow browser therefore costs the
//! core nothing.
//!
//! Sequence numbers are assigned and sent under one lock, so subscribers see
//! them in order. A snapshot is built under the same lock, so the sequence
//! number it carries is never ahead of the state it shows.

use std::sync::{Mutex, PoisonError};

use tokio::sync::broadcast;

use crate::api::{ServerEvent, StateSnapshot};

/// Capacity of the broadcast channel.
pub const EVENT_CAPACITY: usize = 64;

/// The sender side of the stream plus the sequence counter.
#[derive(Debug)]
pub struct EventBus {
    tx: broadcast::Sender<ServerEvent>,
    seq: Mutex<u64>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(EVENT_CAPACITY);
        Self {
            tx,
            seq: Mutex::new(0),
        }
    }

    /// A new receiver. Subscribe before asking for the snapshot, so no event
    /// falls between the two.
    pub fn subscribe(&self) -> broadcast::Receiver<ServerEvent> {
        self.tx.subscribe()
    }

    /// The sequence number of the newest published event, 0 before the first.
    pub fn current_seq(&self) -> u64 {
        *self.seq.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Publishes the event `build` makes from its sequence number and returns
    /// that number. Having no subscriber is normal and not an error.
    pub fn publish(&self, build: impl FnOnce(u64) -> ServerEvent) -> u64 {
        let mut guard = self.seq.lock().unwrap_or_else(PoisonError::into_inner);
        *guard += 1;
        let seq = *guard;
        let _ = self.tx.send(build(seq));
        seq
    }

    /// A `Snapshot` event stamped with the current sequence number. `state`
    /// runs under the sequence lock and must be quick and must not publish.
    pub fn snapshot_event(&self, state: impl FnOnce() -> StateSnapshot) -> ServerEvent {
        let guard = self.seq.lock().unwrap_or_else(PoisonError::into_inner);
        ServerEvent::Snapshot {
            seq: *guard,
            state: state(),
        }
    }
}

#[cfg(test)]
mod tests {
    use tokio::sync::broadcast::error::{RecvError, TryRecvError};

    use super::*;

    fn heartbeat(seq: u64) -> ServerEvent {
        ServerEvent::Heartbeat { seq, uptime_ms: 0 }
    }

    #[test]
    fn sequence_numbers_start_at_one_and_grow_by_one() {
        let bus = EventBus::new();
        assert_eq!(bus.current_seq(), 0);
        let mut rx = bus.subscribe();
        for expected in 1..=3 {
            assert_eq!(bus.publish(heartbeat), expected);
        }
        for expected in 1..=3 {
            assert_eq!(rx.try_recv().expect("event").seq(), expected);
        }
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
        assert_eq!(bus.current_seq(), 3);
    }

    #[test]
    fn publishing_without_a_subscriber_is_not_an_error() {
        let bus = EventBus::new();
        assert_eq!(bus.publish(heartbeat), 1);
    }

    #[test]
    fn a_subscriber_that_falls_behind_is_told_it_lagged_and_the_sender_never_waits() {
        let bus = EventBus::new();
        let mut slow = bus.subscribe();
        let total = u64::try_from(EVENT_CAPACITY).expect("small") + 10;
        for _ in 0..total {
            bus.publish(heartbeat);
        }
        match slow.try_recv() {
            Err(TryRecvError::Lagged(skipped)) => assert_eq!(skipped, 10),
            other => panic!("expected a lag report, got {other:?}"),
        }
        // After the report the oldest retained event follows, not the lost ones.
        assert_eq!(slow.try_recv().expect("event").seq(), 11);
    }

    #[tokio::test]
    async fn concurrent_publishers_deliver_in_sequence_order() {
        let bus = std::sync::Arc::new(EventBus::new());
        let mut rx = bus.subscribe();
        let mut tasks = Vec::new();
        for _ in 0..4 {
            let bus = std::sync::Arc::clone(&bus);
            tasks.push(tokio::spawn(async move {
                for _ in 0..10 {
                    bus.publish(heartbeat);
                }
            }));
        }
        for task in tasks {
            task.await.expect("task");
        }
        let mut last = 0;
        for _ in 0..40 {
            match rx.recv().await {
                Ok(event) => {
                    assert!(event.seq() > last, "{} after {last}", event.seq());
                    last = event.seq();
                }
                Err(RecvError::Lagged(_)) => panic!("capacity is 64, 40 events cannot lag"),
                Err(RecvError::Closed) => panic!("closed"),
            }
        }
        assert_eq!(last, 40);
    }
}
