use crate::{Consumer, Producer, Resampler, ring};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

struct Shared {
    /// Stream position up to which the callback must discard audio. Set by `stop`.
    flush_to: AtomicUsize,
    /// True while more audio for the current turn is expected, so an empty queue is a gap.
    expecting_more: AtomicBool,
    /// True when the last callback played real samples.
    audible: AtomicBool,
    underruns: AtomicU64,
}

/// The side the program uses: queue TTS chunks, stop, read counters.
pub struct Playback {
    producer: Producer,
    resampler: Option<(u32, Resampler)>,
    device_rate: u32,
    shared: Arc<Shared>,
}

/// The side the audio callback uses. `fill` only touches atomics and the ring.
pub struct PlaybackSource {
    consumer: Consumer,
    channels: usize,
    shared: Arc<Shared>,
}

/// `capacity_seconds` bounds the queue. The device rate and channel count come
/// from the output stream that was opened.
pub fn playback(
    device_rate: u32,
    channels: u16,
    capacity_seconds: u32,
) -> (Playback, PlaybackSource) {
    let (producer, consumer) = ring(device_rate as usize * capacity_seconds.max(1) as usize);
    let shared = Arc::new(Shared {
        flush_to: AtomicUsize::new(0),
        expecting_more: AtomicBool::new(false),
        audible: AtomicBool::new(false),
        underruns: AtomicU64::new(0),
    });
    (
        Playback {
            producer,
            resampler: None,
            device_rate,
            shared: Arc::clone(&shared),
        },
        PlaybackSource {
            consumer,
            channels: usize::from(channels.max(1)),
            shared,
        },
    )
}

impl Playback {
    /// Queues one mono chunk at `rate`. Chunks of the same rate share one
    /// resampler, so they join without a seam; the last millisecond or so of a
    /// stream stays inside the filter. Returns false, queuing nothing, when the
    /// chunk does not fit.
    pub fn enqueue(&mut self, samples: &[f32], rate: u32) -> bool {
        if !matches!(&self.resampler, Some((r, _)) if *r == rate) {
            let Some(rs) = Resampler::new(rate, 1, self.device_rate) else {
                return false;
            };
            self.resampler = Some((rate, rs));
        }
        let Some((_, rs)) = self.resampler.as_mut() else {
            return false;
        };
        // A resampler that cannot be rolled back must not run on a chunk we may refuse.
        let worst_case =
            (samples.len() as u64 * u64::from(self.device_rate) / u64::from(rate) + 64) as usize;
        if worst_case > self.producer.free() {
            return false;
        }
        let out = rs.push(samples);
        let stored = self.producer.push(&out);
        self.shared.expecting_more.store(true, Ordering::Relaxed);
        stored == out.len()
    }

    /// Marks the end of the turn's audio. From now on an empty queue is silence, not an underrun.
    pub fn finish_turn(&mut self) {
        self.shared.expecting_more.store(false, Ordering::Relaxed);
    }

    /// Discards everything queued so far. Audio queued after this call plays.
    /// The callback performs the flush, so it completes within one callback period.
    pub fn stop(&mut self) {
        self.resampler = None;
        self.shared.expecting_more.store(false, Ordering::Relaxed);
        self.shared
            .flush_to
            .store(self.producer.position(), Ordering::Release);
    }

    pub fn underruns(&self) -> u64 {
        self.shared.underruns.load(Ordering::Relaxed)
    }

    /// True when the last callback played real audio. Feeds `MicGate::pass`.
    pub fn is_audible(&self) -> bool {
        self.shared.audible.load(Ordering::Relaxed)
    }
}

impl PlaybackSource {
    /// Fills an interleaved output buffer. Missing audio becomes silence.
    pub fn fill(&mut self, out: &mut [f32]) {
        self.consumer
            .skip_to(self.shared.flush_to.load(Ordering::Acquire));
        let frames = out.len() / self.channels;
        let mut sample = [0.0f32; 1];
        let mut played = 0;
        for frame in out.chunks_exact_mut(self.channels) {
            let got = self.consumer.pop_into(&mut sample);
            played += got;
            frame.fill(if got == 1 { sample[0] } else { 0.0 });
        }
        out[frames * self.channels..].fill(0.0);
        let starved = played < frames && self.shared.expecting_more.load(Ordering::Relaxed);
        if starved {
            self.shared.underruns.fetch_add(1, Ordering::Relaxed);
        }
        self.shared.audible.store(played > 0, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DC: f32 = 0.25;

    fn chunk(rate: u32, ms: u32) -> Vec<f32> {
        vec![DC; (rate * ms / 1000) as usize]
    }

    #[test]
    fn queued_audio_plays_in_order_and_fans_out_to_every_channel() {
        let (mut p, mut src) = playback(16_000, 2, 5);
        assert!(p.enqueue(&chunk(16_000, 200), 16_000));
        let mut out = vec![0.0; 2 * 800];
        src.fill(&mut out);
        assert!(p.is_audible());
        assert!(
            out[1000..1100].iter().all(|v| (v - DC).abs() < 1e-3),
            "{:?}",
            &out[1000..1004]
        );
    }

    #[test]
    fn a_24k_chunk_is_converted_to_the_device_rate() {
        let (mut p, mut src) = playback(48_000, 1, 5);
        assert!(p.enqueue(&chunk(24_000, 500), 24_000));
        let mut out = vec![0.0; 24_000 - 100];
        src.fill(&mut out);
        let played = out.iter().filter(|v| **v != 0.0).count();
        assert!(played > 23_000, "{played}");
    }

    #[test]
    fn stop_discards_queued_audio_but_not_audio_queued_after_it() {
        let (mut p, mut src) = playback(16_000, 1, 5);
        p.enqueue(&chunk(16_000, 1000), 16_000);
        p.stop();
        p.enqueue(&chunk(16_000, 400), 16_000);
        let mut out = vec![0.0; 16_000];
        src.fill(&mut out);
        let played = out.iter().filter(|v| **v != 0.0).count();
        // 400 ms minus the filter's held-back tail, and nothing from the first second.
        assert!((6_000..=6_400).contains(&played), "{played}");
        src.fill(&mut out);
        assert!(out.iter().all(|v| *v == 0.0));
    }

    #[test]
    fn a_gap_during_a_turn_is_an_underrun_and_the_end_of_a_turn_is_not() {
        let (mut p, mut src) = playback(16_000, 1, 5);
        p.enqueue(&chunk(16_000, 100), 16_000);
        let mut out = vec![0.0; 3200];
        src.fill(&mut out);
        assert_eq!(p.underruns(), 1);
        p.finish_turn();
        src.fill(&mut out);
        assert_eq!(p.underruns(), 1);
        assert!(!p.is_audible());
    }

    #[test]
    fn a_chunk_that_does_not_fit_is_refused_whole() {
        let (mut p, mut src) = playback(16_000, 1, 1);
        assert!(p.enqueue(&chunk(16_000, 800), 16_000));
        assert!(!p.enqueue(&chunk(16_000, 800), 16_000));
        let mut out = vec![0.0; 16_000];
        src.fill(&mut out);
        assert!(out.iter().filter(|v| **v != 0.0).count() < 13_000);
    }
}
