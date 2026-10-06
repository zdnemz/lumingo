use std::sync::{
    Arc,
    atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering},
};

struct Shared {
    /// `f32` bit patterns, so the buffer needs no `unsafe` and no lock.
    slots: Box<[AtomicU32]>,
    /// Total samples ever written. Only the producer stores to it.
    head: AtomicUsize,
    /// Total samples ever read. Only the consumer stores to it.
    tail: AtomicUsize,
    /// Samples the producer had to drop because the buffer was full.
    overflowed: AtomicU64,
}

/// Capture side of the ring. `push` is for the audio callback: it copies and
/// returns. It never blocks, locks, allocates or logs. When the buffer is full the
/// samples that do not fit are dropped and counted.
pub struct Producer(Arc<Shared>);

/// Reader side, owned by the VAD worker thread.
pub struct Consumer(Arc<Shared>);

/// Creates a ring that holds `capacity` samples. A capacity of zero is raised to one.
pub fn ring(capacity: usize) -> (Producer, Consumer) {
    let shared = Arc::new(Shared {
        slots: (0..capacity.max(1)).map(|_| AtomicU32::new(0)).collect(),
        head: AtomicUsize::new(0),
        tail: AtomicUsize::new(0),
        overflowed: AtomicU64::new(0),
    });
    (Producer(Arc::clone(&shared)), Consumer(shared))
}

impl Producer {
    /// Returns how many samples were stored. The rest are counted as overflow.
    pub fn push(&mut self, samples: &[f32]) -> usize {
        let s = &*self.0;
        let cap = s.slots.len();
        let head = s.head.load(Ordering::Relaxed);
        let free = cap - head.wrapping_sub(s.tail.load(Ordering::Acquire));
        let n = samples.len().min(free);
        for (i, v) in samples[..n].iter().enumerate() {
            s.slots[head.wrapping_add(i) % cap].store(v.to_bits(), Ordering::Relaxed);
        }
        s.head.store(head.wrapping_add(n), Ordering::Release);
        if n < samples.len() {
            s.overflowed
                .fetch_add((samples.len() - n) as u64, Ordering::Relaxed);
        }
        n
    }

    pub fn overflowed(&self) -> u64 {
        self.0.overflowed.load(Ordering::Relaxed)
    }

    /// Samples that fit right now.
    pub fn free(&self) -> usize {
        let s = &*self.0;
        s.slots.len()
            - s.head
                .load(Ordering::Relaxed)
                .wrapping_sub(s.tail.load(Ordering::Acquire))
    }

    /// Total samples ever written. Pair it with `Consumer::position` to name a point in the stream.
    pub fn position(&self) -> usize {
        self.0.head.load(Ordering::Relaxed)
    }
}

impl Consumer {
    /// Moves up to `out.len()` samples into `out` and returns how many.
    pub fn pop_into(&mut self, out: &mut [f32]) -> usize {
        let s = &*self.0;
        let cap = s.slots.len();
        let tail = s.tail.load(Ordering::Relaxed);
        let available = s.head.load(Ordering::Acquire).wrapping_sub(tail);
        let n = out.len().min(available);
        for (i, slot) in out[..n].iter_mut().enumerate() {
            *slot = f32::from_bits(s.slots[tail.wrapping_add(i) % cap].load(Ordering::Relaxed));
        }
        s.tail.store(tail.wrapping_add(n), Ordering::Release);
        n
    }

    /// Total samples ever read or skipped.
    pub fn position(&self) -> usize {
        self.0.tail.load(Ordering::Relaxed)
    }

    /// Discards samples up to the absolute stream position `target`, at most what is buffered.
    pub fn skip_to(&mut self, target: usize) {
        let s = &*self.0;
        let tail = s.tail.load(Ordering::Relaxed);
        let head = s.head.load(Ordering::Acquire);
        let want = target.wrapping_sub(tail);
        if want != 0 && want <= head.wrapping_sub(tail) {
            s.tail.store(target, Ordering::Release);
        } else if want != 0 && (want as isize) > 0 {
            s.tail.store(head, Ordering::Release);
        }
    }

    pub fn len(&self) -> usize {
        let s = &*self.0;
        s.head
            .load(Ordering::Acquire)
            .wrapping_sub(s.tail.load(Ordering::Relaxed))
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn overflowed(&self) -> u64 {
        self.0.overflowed.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overfilling_drops_the_excess_counts_it_and_never_blocks() {
        let (mut p, mut c) = ring(8);
        assert_eq!(p.push(&[1.0; 5]), 5);
        assert_eq!(p.push(&[2.0; 5]), 3);
        assert_eq!(p.overflowed(), 2);
        assert_eq!(c.overflowed(), 2);
        let mut out = [0.0; 16];
        assert_eq!(c.pop_into(&mut out), 8);
        assert_eq!(&out[..8], &[1.0, 1.0, 1.0, 1.0, 1.0, 2.0, 2.0, 2.0]);
        assert!(c.is_empty());
    }

    #[test]
    fn indices_wrap_around_the_end_of_the_buffer() {
        let (mut p, mut c) = ring(4);
        let mut out = [0.0; 3];
        for round in 0..10 {
            let v = round as f32;
            assert_eq!(p.push(&[v, v + 0.5, v + 0.25]), 3);
            assert_eq!(c.pop_into(&mut out), 3);
            assert_eq!(out, [v, v + 0.5, v + 0.25]);
        }
        assert_eq!(p.overflowed(), 0);
    }

    #[test]
    fn a_short_read_leaves_the_rest_for_later() {
        let (mut p, mut c) = ring(8);
        p.push(&[1.0, 2.0, 3.0]);
        let mut one = [0.0; 1];
        assert_eq!(c.pop_into(&mut one), 1);
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn two_threads_see_every_sample_once_and_in_order() {
        let (mut p, mut c) = ring(256);
        const TOTAL: usize = 200_000;
        let writer = std::thread::spawn(move || {
            let mut next = 0;
            while next < TOTAL {
                let end = (next + 100).min(TOTAL);
                let chunk: Vec<f32> = (next..end).map(|i| i as f32).collect();
                next += p.push(&chunk);
                // Retry from `next` instead of dropping, so the sequence stays checkable.
                if next < end {
                    std::thread::yield_now();
                }
            }
        });
        let mut seen = 0usize;
        let mut buf = [0.0; 64];
        while seen < TOTAL {
            let n = c.pop_into(&mut buf);
            for v in &buf[..n] {
                assert_eq!(*v, seen as f32);
                seen += 1;
            }
        }
        writer.join().unwrap();
    }
}
