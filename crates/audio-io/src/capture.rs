//! The capture path: ring buffer to gated 16 kHz frames.
//!
//! [`CapturePath`] is plain logic that a worker thread calls in a loop. It reads
//! device samples from the ring, converts them to 16 kHz mono, cuts them into
//! fixed-size frames (the size the VAD wants), asks the [`MicGate`] where each
//! frame goes and hands it to the caller. It owns no thread and touches no
//! device, so tests drive it directly.

use crate::gate::{MicGate, Route};
use crate::resample::{CaptureConverter, ResampleError};
use crate::ring::CaptureConsumer;

/// Device samples read from the ring per step. Bounded so one step stays short.
const READ_BLOCK: usize = 8192;

/// What one call to [`CapturePath::pump`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pumped {
    /// Frames passed to the sink, whatever their route.
    pub frames: usize,
    /// The stream behind the ring is gone and the ring is empty.
    pub input_closed: bool,
}

pub struct CapturePath {
    consumer: CaptureConsumer,
    converter: CaptureConverter,
    gate: MicGate,
    frame_len: usize,
    read_buf: Vec<f32>,
    speech: Vec<f32>,
    framer: Vec<f32>,
}

impl std::fmt::Debug for CapturePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CapturePath")
            .field("frame_len", &self.frame_len)
            .finish_non_exhaustive()
    }
}

impl CapturePath {
    /// `frame_len` is in 16 kHz samples. A zero is treated as one.
    pub fn new(
        consumer: CaptureConsumer,
        converter: CaptureConverter,
        gate: MicGate,
        frame_len: usize,
    ) -> Self {
        let frame_len = frame_len.max(1);
        Self {
            consumer,
            converter,
            gate,
            frame_len,
            read_buf: vec![0.0; READ_BLOCK],
            speech: Vec::new(),
            framer: Vec::with_capacity(frame_len * 2),
        }
    }

    /// Switches to a new input stream, for example after a device change. Audio
    /// left over from the old stream is discarded so no frame mixes two devices.
    pub fn replace_input(&mut self, consumer: CaptureConsumer, converter: CaptureConverter) {
        self.consumer = consumer;
        self.converter = converter;
        self.framer.clear();
    }

    /// Moves everything currently in the ring through conversion, framing and the
    /// gate. `sink` receives each complete frame with its route.
    pub fn pump(&mut self, sink: &mut dyn FnMut(&[f32], Route)) -> Result<Pumped, ResampleError> {
        let mut frames = 0;
        loop {
            let read = self.consumer.pop_into(&mut self.read_buf);
            if read == 0 {
                break;
            }
            self.speech.clear();
            self.converter
                .process(&self.read_buf[..read], &mut self.speech)?;
            self.framer.extend_from_slice(&self.speech);
            let whole = self.framer.len() / self.frame_len * self.frame_len;
            for frame in self.framer[..whole].chunks_exact(self.frame_len) {
                let route = self.gate.route(frame.len());
                sink(frame, route);
                frames += 1;
            }
            self.framer.drain(..whole);
        }
        Ok(Pumped {
            frames,
            input_closed: self.consumer.is_abandoned() && self.consumer.available() == 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gate::PushToTalk;
    use crate::ring::{CaptureProducer, capture_ring};
    use std::f32::consts::PI;
    use std::num::NonZeroUsize;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    const FRAME: usize = 512;

    struct Rig {
        producer: CaptureProducer,
        path: CapturePath,
        playing: Arc<AtomicBool>,
        ptt: PushToTalk,
    }

    fn rig(rate: u32, channels: u16) -> Rig {
        let (producer, consumer, _counters) =
            capture_ring(NonZeroUsize::new(1 << 20).expect("non-zero"));
        let playing = Arc::new(AtomicBool::new(false));
        let ptt = PushToTalk::new();
        let gate = MicGate::new(playing.clone(), ptt.clone(), 16_000);
        let converter = CaptureConverter::new(rate, channels).expect("converter builds");
        Rig {
            producer,
            path: CapturePath::new(consumer, converter, gate, FRAME),
            playing,
            ptt,
        }
    }

    /// `ms` of a loud 300 Hz tone, standing in for a recorded WAV of speech.
    fn voice(rate: u32, channels: usize, ms: u32) -> Vec<f32> {
        let n = (u64::from(rate) * u64::from(ms) / 1000) as usize;
        (0..n)
            .flat_map(|i| {
                let s = 0.5 * (2.0 * PI * 300.0 * i as f32 / rate as f32).sin();
                std::iter::repeat_n(s, channels)
            })
            .collect()
    }

    fn silence(rate: u32, channels: usize, ms: u32) -> Vec<f32> {
        vec![0.0; (u64::from(rate) * u64::from(ms) / 1000) as usize * channels]
    }

    /// A stand-in for the real VAD plus endpointing: an utterance is a run of
    /// loud frames that ends at a quiet one. Only used to count utterance events.
    #[derive(Default)]
    struct UtteranceCounter {
        in_speech: bool,
        utterances: usize,
        vad_frames: usize,
        ptt_frames: usize,
    }

    impl UtteranceCounter {
        fn feed(&mut self, frame: &[f32], route: Route) {
            match route {
                Route::Dropped => {}
                Route::PushToTalk => self.ptt_frames += 1,
                Route::Vad => {
                    self.vad_frames += 1;
                    let rms =
                        (frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32).sqrt();
                    let loud = rms > 0.05;
                    if self.in_speech && !loud {
                        self.utterances += 1;
                    }
                    self.in_speech = loud;
                }
            }
        }
    }

    fn pump(rig: &mut Rig, counter: &mut UtteranceCounter) {
        let result = rig
            .path
            .pump(&mut |frame, route| counter.feed(frame, route));
        assert!(result.is_ok());
    }

    #[test]
    fn a_recording_played_into_capture_during_playback_makes_no_utterance() {
        let mut rig = rig(48_000, 2);
        let mut counter = UtteranceCounter::default();

        // The tutor is speaking; the microphone hears it (the "WAV") and then
        // the speaker stops while the room still rings for a moment.
        rig.playing.store(true, Ordering::Release);
        assert!(rig.producer.push(&voice(48_000, 2, 1_000)));
        pump(&mut rig, &mut counter);
        rig.playing.store(false, Ordering::Release);
        assert!(rig.producer.push(&voice(48_000, 2, 100)));
        assert!(rig.producer.push(&silence(48_000, 2, 100)));
        pump(&mut rig, &mut counter);
        assert_eq!(counter.utterances, 0, "no utterance event from tutor audio");

        // After the hold the gate is open, and real speech is heard again.
        assert!(rig.producer.push(&silence(48_000, 2, 300)));
        assert!(rig.producer.push(&voice(48_000, 2, 600)));
        assert!(rig.producer.push(&silence(48_000, 2, 300)));
        pump(&mut rig, &mut counter);
        assert_eq!(counter.utterances, 1);
    }

    #[test]
    fn speech_right_after_playback_is_dropped_for_150_ms_and_no_longer() {
        let mut rig = rig(16_000, 1);
        let mut counter = UtteranceCounter::default();
        rig.playing.store(true, Ordering::Release);
        assert!(rig.producer.push(&silence(16_000, 1, 64)));
        pump(&mut rig, &mut counter);
        rig.playing.store(false, Ordering::Release);

        // 1 s of samples at 16 kHz is 31 whole frames of 512; count what the VAD gets.
        assert!(rig.producer.push(&voice(16_000, 1, 1_000)));
        pump(&mut rig, &mut counter);
        let gate_closed_frames = 31 - counter.vad_frames;
        // 150 ms is 2400 samples, which is five 512-sample frames.
        assert_eq!(gate_closed_frames, 5);
    }

    #[test]
    fn push_to_talk_audio_takes_its_own_route_during_playback() {
        let mut rig = rig(16_000, 1);
        let mut counter = UtteranceCounter::default();
        rig.playing.store(true, Ordering::Release);
        rig.ptt.set(true);
        assert!(rig.producer.push(&voice(16_000, 1, 500)));
        pump(&mut rig, &mut counter);
        assert_eq!(counter.vad_frames, 0, "the VAD never hears push-to-talk");
        assert_eq!(counter.ptt_frames, 15);
    }

    #[test]
    fn frames_are_always_full_and_odd_pushes_do_not_lose_samples() {
        let mut rig = rig(44_100, 2);
        let mut sizes = Vec::new();
        let input = voice(44_100, 2, 2_000);
        for piece in input.chunks(1_001 * 2) {
            assert!(rig.producer.push(piece));
            let r = rig.path.pump(&mut |frame, _| sizes.push(frame.len()));
            assert!(r.is_ok());
        }
        assert!(sizes.iter().all(|n| *n == FRAME));
        // 2 s at 44.1 kHz is 32 000 samples at 16 kHz; the filter delay is trimmed
        // and the last partial frame waits for more audio.
        let total = sizes.len() * FRAME;
        assert!(total <= 32_000 && total + 2 * FRAME > 32_000, "{total}");
    }

    #[test]
    fn a_closed_stream_is_reported_once_the_ring_is_drained() {
        let mut rig = rig(16_000, 1);
        let mut counter = UtteranceCounter::default();
        assert!(rig.producer.push(&voice(16_000, 1, 100)));
        let Rig {
            producer, mut path, ..
        } = rig;
        drop(producer);
        let first = path.pump(&mut |frame, route| counter.feed(frame, route));
        assert_eq!(first.map(|p| p.input_closed).ok(), Some(true));
    }

    #[test]
    fn replacing_the_input_switches_rate_without_mixing_devices() {
        let mut rig = rig(48_000, 2);
        let mut counter = UtteranceCounter::default();
        assert!(rig.producer.push(&voice(48_000, 2, 100)));
        pump(&mut rig, &mut counter);

        let (mut producer, consumer, _c) =
            capture_ring(NonZeroUsize::new(1 << 16).expect("non-zero"));
        let converter = CaptureConverter::new(16_000, 1).expect("builds");
        rig.path.replace_input(consumer, converter);
        assert!(producer.push(&voice(16_000, 1, 200)));
        let mut frames = 0;
        let result = rig.path.pump(&mut |_, _| frames += 1);
        assert!(result.is_ok());
        assert_eq!(
            frames, 6,
            "200 ms at 16 kHz is 3200 samples, six full frames"
        );
    }
}
