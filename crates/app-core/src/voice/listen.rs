//! The listening side: gated frames in, utterances out.
//!
//! The capture worker of `audio-io` calls [`frame_sink`] for every 16 kHz frame
//! with the microphone gate's decision. The sink copies the frame into a bounded
//! queue (`VoiceConfig::frame_queue`, 256 frames by default). When the queue is
//! full the frame is dropped and counted: the capture worker must never wait,
//! because a slow sink fills the capture ring and loses audio there instead.
//!
//! A `vad` thread reads that queue. [`ListenState`] is its logic, kept free of
//! threads so tests drive it frame by frame:
//!
//! * `Route::Vad` frames go through the VAD and the segmenter. Every frame
//!   that counts as speech stamps the loop clock, so the latency starts from the
//!   last speech sample and not from when the endpointer finally decided.
//! * `Route::Dropped` marks the gate closing (tutor audio, plus 150 ms after).
//!   The VAD and the segmenter are reset so no utterance spans the gap.
//! * `Route::PushToTalk` frames bypass the VAD. They are collected until the
//!   learner lets go, and then form one utterance. A tap shorter than the
//!   minimum speech length is ignored.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::thread::JoinHandle;
use std::time::Duration;

use audio_io::{FrameSink, Route};
use speech::{CancelFlag, SPEECH_SAMPLE_RATE, SegmentEvent, UtteranceSegmenter, Vad};

use super::clock::LoopClock;
use super::msg::{Counters, Inbound, Inbox, ListenEvent, UtteranceMsg};

/// The longest push-to-talk utterance kept, the same 30 s as the endpointer's forced end.
const MAX_PTT_SAMPLES: usize = 30 * SPEECH_SAMPLE_RATE as usize;

/// How often the thread looks at the cancel flag while the queue is empty.
const IDLE_WAIT: Duration = Duration::from_millis(50);

/// The loudest sample heard since the level was last read. The listener thread
/// writes it for every frame it handles; whoever shows a level meter reads it a
/// few times a second. Both sides are one atomic: nothing waits, and a read that
/// comes between two frames still sees the peak of the earlier one.
#[derive(Debug, Clone, Default)]
pub(crate) struct LevelMeter(Arc<AtomicU32>);

impl LevelMeter {
    /// Folds the peak of `frame` in. The bit pattern of a non-negative `f32` sorts
    /// like the number, so `fetch_max` on the bits keeps the larger value.
    pub(crate) fn record(&self, frame: &[f32]) {
        let peak = frame
            .iter()
            .fold(0.0_f32, |peak, sample| peak.max(sample.abs()))
            .min(1.0);
        self.0.fetch_max(peak.to_bits(), Ordering::Relaxed);
    }

    /// The peak since the last call, 0 to 1, and starts a new window.
    pub(crate) fn take(&self) -> f32 {
        f32::from_bits(self.0.swap(0, Ordering::Relaxed))
    }
}

pub(crate) struct FrameMsg {
    /// Empty for the marker that says the gate closed.
    pub frame: Vec<f32>,
    pub route: Route,
}

/// The sink to start the audio session with.
pub(crate) fn frame_sink(tx: SyncSender<FrameMsg>, counters: Arc<Counters>) -> FrameSink {
    let mut gate_was_closed = false;
    Box::new(move |frame, route| {
        let message = match route {
            Route::Dropped => {
                // One marker per closing is enough: the listener resets on it.
                if gate_was_closed {
                    return;
                }
                gate_was_closed = true;
                FrameMsg {
                    frame: Vec::new(),
                    route,
                }
            }
            Route::Vad | Route::PushToTalk => {
                gate_was_closed = false;
                FrameMsg {
                    frame: frame.to_vec(),
                    route,
                }
            }
        };
        match tx.try_send(message) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(_)) => Counters::bump(&counters.frames_dropped),
        }
    })
}

pub(crate) struct ListenState {
    vad: Box<dyn Vad>,
    segmenter: UtteranceSegmenter,
    clock: Arc<dyn LoopClock>,
    speech_threshold: f32,
    silence_threshold: f32,
    min_ptt_samples: usize,
    last_speech: Option<Duration>,
    vad_live: bool,
    ptt: Option<PttCapture>,
    counters: Arc<Counters>,
    level: LevelMeter,
}

struct PttCapture {
    samples: Vec<f32>,
    last_frame: Duration,
}

impl ListenState {
    pub(crate) fn new(
        vad: Box<dyn Vad>,
        segmenter: UtteranceSegmenter,
        clock: Arc<dyn LoopClock>,
        counters: Arc<Counters>,
        level: LevelMeter,
    ) -> Self {
        let config = segmenter.config();
        let speech_threshold = config.speech_threshold;
        let silence_threshold = config.silence_threshold;
        let min_ptt_samples =
            (u64::from(config.min_speech_ms) * u64::from(SPEECH_SAMPLE_RATE) / 1000) as usize;
        Self {
            vad,
            segmenter,
            clock,
            speech_threshold,
            silence_threshold,
            min_ptt_samples,
            last_speech: None,
            vad_live: false,
            ptt: None,
            counters,
            level,
        }
    }

    /// Handles one message from the queue and returns what the orchestrator should hear.
    pub(crate) fn on_frame(&mut self, message: FrameMsg) -> Vec<ListenEvent> {
        let mut out = Vec::new();
        if !message.frame.is_empty() {
            self.level.record(&message.frame);
        }
        match message.route {
            Route::Dropped => {
                self.finish_ptt(&mut out);
                if self.vad_live {
                    self.reset_vad();
                }
            }
            Route::PushToTalk => self.on_ptt(message.frame, &mut out),
            Route::Vad => {
                self.finish_ptt(&mut out);
                self.on_vad(&message.frame, &mut out);
            }
        }
        out
    }

    fn reset_vad(&mut self) {
        self.vad.reset();
        self.segmenter.reset();
        self.last_speech = None;
        self.vad_live = false;
    }

    fn on_ptt(&mut self, frame: Vec<f32>, out: &mut Vec<ListenEvent>) {
        if self.ptt.is_none() {
            // The learner took the floor: whatever the VAD was following is over.
            self.reset_vad();
            self.ptt = Some(PttCapture {
                samples: Vec::new(),
                last_frame: self.clock.now(),
            });
            out.push(ListenEvent::PushToTalkStarted);
        }
        if let Some(capture) = self.ptt.as_mut() {
            if capture.samples.len() + frame.len() > MAX_PTT_SAMPLES {
                Counters::bump(&self.counters.ptt_dropped);
            } else {
                capture.samples.extend_from_slice(&frame);
            }
            capture.last_frame = self.clock.now();
        }
    }

    fn finish_ptt(&mut self, out: &mut Vec<ListenEvent>) {
        let Some(capture) = self.ptt.take() else {
            return;
        };
        if capture.samples.len() < self.min_ptt_samples {
            return;
        }
        out.push(ListenEvent::Utterance(UtteranceMsg {
            samples: capture.samples,
            last_speech: capture.last_frame,
            ended: capture.last_frame,
        }));
    }

    fn on_vad(&mut self, frame: &[f32], out: &mut Vec<ListenEvent>) {
        self.vad_live = true;
        let probability = match self.vad.push_frame(frame) {
            Ok(p) => p,
            Err(error) => {
                out.push(ListenEvent::Failed(error.to_string()));
                return;
            }
        };
        // The endpointer counts a frame as speech by the same two thresholds.
        let threshold = if self.segmenter.is_speaking() {
            self.silence_threshold
        } else {
            self.speech_threshold
        };
        let now = self.clock.now();
        if probability >= threshold {
            self.last_speech = Some(now);
        }
        match self.segmenter.push_frame(probability, frame) {
            None => {}
            Some(SegmentEvent::SpeechStarted) => out.push(ListenEvent::SpeechStarted),
            Some(SegmentEvent::Utterance(utterance)) => {
                out.push(ListenEvent::Utterance(UtteranceMsg {
                    samples: utterance.samples,
                    last_speech: self.last_speech.unwrap_or(now),
                    ended: now,
                }));
                self.last_speech = None;
            }
        }
    }
}

/// Runs the listener on its own thread until the flag is set or the queue is closed.
pub(crate) fn spawn_listener(
    mut state: ListenState,
    frames: Receiver<FrameMsg>,
    inbox: Inbox,
    cancel: CancelFlag,
) -> std::io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("lumingo-vad".to_owned())
        .spawn(move || {
            while !cancel.is_cancelled() {
                match frames.recv_timeout(IDLE_WAIT) {
                    Ok(message) => {
                        for event in state.on_frame(message) {
                            inbox.post(Inbound::Listen(event));
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        })
}
