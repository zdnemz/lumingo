//! The capture ring buffer: the only thing an audio callback touches.
//!
//! The device callback owns a [`CaptureProducer`]. It copies one block of
//! interleaved samples into a lock-free single-producer single-consumer queue
//! and returns. It never waits: when the queue cannot take the whole block, the
//! block is dropped and the overflow counters go up. The worker thread owns the
//! [`CaptureConsumer`] and reads at its own pace.
//!
//! A block is written whole or not at all. Writing half of a block would cut an
//! interleaved frame in two and shift every later channel by one sample.

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Default)]
struct Counters {
    pushed_samples: AtomicU64,
    dropped_samples: AtomicU64,
    overflow_events: AtomicU64,
}

/// A point-in-time copy of the capture counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CaptureSnapshot {
    /// Samples that reached the ring.
    pub pushed_samples: u64,
    /// Samples that were thrown away because the ring was full.
    pub dropped_samples: u64,
    /// Callback blocks that were thrown away. Zero is the target.
    pub overflow_events: u64,
}

/// A cheap, cloneable view of the counters. Reading never blocks the callback.
#[derive(Debug, Clone)]
pub struct CaptureCounters(Arc<Counters>);

impl CaptureCounters {
    pub fn snapshot(&self) -> CaptureSnapshot {
        CaptureSnapshot {
            pushed_samples: self.0.pushed_samples.load(Ordering::Relaxed),
            dropped_samples: self.0.dropped_samples.load(Ordering::Relaxed),
            overflow_events: self.0.overflow_events.load(Ordering::Relaxed),
        }
    }
}

/// The audio-callback end of the capture ring.
#[derive(Debug)]
pub struct CaptureProducer {
    inner: rtrb::Producer<f32>,
    counters: Arc<Counters>,
}

impl CaptureProducer {
    /// Copies `block` into the ring. Returns `false` and counts an overflow when
    /// the whole block does not fit. No allocation, no lock, no waiting.
    pub fn push(&mut self, block: &[f32]) -> bool {
        match self.inner.push_entire_slice(block) {
            Ok(()) => {
                self.counters
                    .pushed_samples
                    .fetch_add(block.len() as u64, Ordering::Relaxed);
                true
            }
            Err(_) => {
                self.counters
                    .dropped_samples
                    .fetch_add(block.len() as u64, Ordering::Relaxed);
                self.counters
                    .overflow_events
                    .fetch_add(1, Ordering::Relaxed);
                false
            }
        }
    }
}

/// The worker end of the capture ring.
#[derive(Debug)]
pub struct CaptureConsumer {
    inner: rtrb::Consumer<f32>,
}

impl CaptureConsumer {
    /// Samples ready to read right now.
    pub fn available(&self) -> usize {
        self.inner.slots()
    }

    /// Copies up to `out.len()` samples into `out` and returns how many.
    pub fn pop_into(&mut self, out: &mut [f32]) -> usize {
        let (popped, _) = self.inner.pop_partial_slice(out);
        popped.len()
    }

    /// True once the callback side was dropped, which is what a closed or lost
    /// stream looks like from here. Samples still in the ring can be read first.
    pub fn is_abandoned(&self) -> bool {
        self.inner.is_abandoned()
    }
}

/// Creates a ring that holds `capacity_samples` interleaved samples.
///
/// Size it for the longest stall you want to survive in the worker, for example
/// two seconds of `rate * channels`.
pub fn capture_ring(
    capacity_samples: NonZeroUsize,
) -> (CaptureProducer, CaptureConsumer, CaptureCounters) {
    let (producer, consumer) = rtrb::RingBuffer::new(capacity_samples.get());
    let counters = Arc::new(Counters::default());
    (
        CaptureProducer {
            inner: producer,
            counters: Arc::clone(&counters),
        },
        CaptureConsumer { inner: consumer },
        CaptureCounters(counters),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring(capacity: usize) -> (CaptureProducer, CaptureConsumer, CaptureCounters) {
        capture_ring(NonZeroUsize::new(capacity).expect("test capacity is non-zero"))
    }

    #[test]
    fn samples_come_out_in_order() {
        let (mut tx, mut rx, counters) = ring(8);
        assert!(tx.push(&[1.0, 2.0, 3.0]));
        assert!(tx.push(&[4.0, 5.0]));
        let mut out = [0.0; 8];
        assert_eq!(rx.pop_into(&mut out), 5);
        assert_eq!(&out[..5], &[1.0, 2.0, 3.0, 4.0, 5.0]);
        assert_eq!(counters.snapshot().pushed_samples, 5);
        assert_eq!(counters.snapshot().overflow_events, 0);
    }

    #[test]
    fn a_fake_callback_that_overfills_the_ring_never_blocks_and_is_counted() {
        let (mut tx, rx, counters) = ring(100);
        // Ten callbacks of 30 samples into a ring of 100 with nobody reading:
        // three fit, seven are dropped whole.
        let block = [0.5_f32; 30];
        let accepted = (0..10).filter(|_| tx.push(&block)).count();
        assert_eq!(accepted, 3);
        let snap = counters.snapshot();
        assert_eq!(snap.pushed_samples, 90);
        assert_eq!(snap.dropped_samples, 210);
        assert_eq!(snap.overflow_events, 7);
        assert_eq!(rx.available(), 90);
    }

    #[test]
    fn a_dropped_block_never_leaves_half_a_frame_behind() {
        let (mut tx, mut rx, _counters) = ring(5);
        assert!(tx.push(&[1.0, 2.0, 3.0, 4.0]));
        // Stereo block of two frames does not fit in the one free slot.
        assert!(!tx.push(&[9.0, 9.0, 9.0, 9.0]));
        let mut out = [0.0; 8];
        assert_eq!(rx.pop_into(&mut out), 4);
        assert_eq!(&out[..4], &[1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn the_ring_recovers_after_the_reader_catches_up() {
        let (mut tx, mut rx, counters) = ring(4);
        assert!(tx.push(&[1.0; 4]));
        assert!(!tx.push(&[2.0; 2]));
        let mut out = [0.0; 4];
        assert_eq!(rx.pop_into(&mut out), 4);
        assert!(tx.push(&[3.0; 4]));
        assert_eq!(counters.snapshot().overflow_events, 1);
    }

    #[test]
    fn dropping_the_callback_side_is_visible_to_the_reader() {
        let (mut tx, mut rx, _counters) = ring(4);
        assert!(tx.push(&[1.0, 2.0]));
        assert!(!rx.is_abandoned());
        drop(tx);
        assert!(rx.is_abandoned());
        let mut out = [0.0; 4];
        assert_eq!(rx.pop_into(&mut out), 2);
    }

    #[test]
    fn a_real_producer_thread_and_a_slow_reader_lose_nothing_that_was_accepted() {
        let (mut tx, mut rx, counters) = ring(1024);
        let writer = std::thread::spawn(move || {
            let mut next = 0.0_f32;
            let mut accepted = Vec::new();
            for _ in 0..2000 {
                let block: Vec<f32> = (0..16).map(|i| next + i as f32).collect();
                if tx.push(&block) {
                    accepted.extend_from_slice(&block);
                }
                next += 16.0;
                std::thread::yield_now();
            }
            accepted
        });
        let mut read = Vec::new();
        let mut buf = [0.0_f32; 64];
        while !(writer.is_finished() && rx.available() == 0) {
            let n = rx.pop_into(&mut buf);
            read.extend_from_slice(&buf[..n]);
            std::thread::sleep(std::time::Duration::from_micros(50));
        }
        let accepted = writer.join().expect("writer thread finished");
        assert_eq!(read, accepted);
        let snap = counters.snapshot();
        assert_eq!(snap.pushed_samples as usize, accepted.len());
        assert_eq!(
            snap.dropped_samples + snap.pushed_samples,
            2000 * 16,
            "every sample is either pushed or counted as dropped"
        );
    }
}
