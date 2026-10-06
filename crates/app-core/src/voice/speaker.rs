//! The speech output side: the TTS worker's events become queued audio.
//!
//! The TTS worker calls the sink [`Speaker::sink`] on its own thread for every
//! event. Audio goes straight into the playback queue from there, so a sentence
//! is audible as soon as it is synthesised and never waits for the Tokio task
//! that reads the LLM stream. The orchestrator only hears a small note about
//! it.
//!
//! Which turn's audio may still play is decided here, under one lock. A turn
//! calls [`Speaker::begin_turn`]; when it is cancelled the orchestrator calls
//! [`Speaker::end_turn`] and only then flushes the playback queue. Audio of any
//! other turn that arrives later is dropped, so a sentence that was being
//! synthesised when the learner interrupted cannot play after the flush.
//!
//! Queues: the notes of one turn go through a channel of 32 (a reply has at most
//! a handful of sentences). When it is full the note is dropped and counted in
//! `VoiceStats::notes_dropped`; the turn then ends by the TTS timeout instead
//! of hanging. A playback queue that has no room for a chunk (30 s of audio
//! by default) is not an error here: the sink hands the event back, the worker
//! offers it again every 5 ms, and the worker waits meanwhile.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use audio_io::{EnqueueOutcome, PlaybackError};
use speech::{TtsEvent, TtsSinkResult};
use tokio::sync::mpsc;

use super::clock::LoopClock;
use super::msg::{Counters, Inbound, Inbox, TtsLifecycle};
use super::port::PlaybackPort;

/// What the notes channel of one turn holds, at most.
pub(crate) const NOTES_CAPACITY: usize = 32;

/// The end of one sentence, as the turn task needs to know it.
#[derive(Debug)]
pub(crate) enum TtsNote {
    /// The audio is in the playback queue. `at` is the loop clock then.
    Audio { at: Duration },
    /// Synthesis or queueing failed.
    Failed { message: String },
    /// The sentence was cancelled before it was synthesised.
    Cancelled,
}

struct Link {
    epoch: u64,
    notes: Option<mpsc::Sender<TtsNote>>,
}

pub(crate) struct Speaker {
    link: Mutex<Link>,
    playback: Arc<dyn PlaybackPort>,
    clock: Arc<dyn LoopClock>,
    inbox: Inbox,
    counters: Arc<Counters>,
}

impl Speaker {
    pub(crate) fn new(
        playback: Arc<dyn PlaybackPort>,
        clock: Arc<dyn LoopClock>,
        inbox: Inbox,
        counters: Arc<Counters>,
    ) -> Self {
        Self {
            link: Mutex::new(Link {
                epoch: 0,
                notes: None,
            }),
            playback,
            clock,
            inbox,
            counters,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Link> {
        self.link.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Lets the audio of turn `epoch` through and returns where its notes arrive.
    pub(crate) fn begin_turn(&self, epoch: u64) -> mpsc::Receiver<TtsNote> {
        let (tx, rx) = mpsc::channel(NOTES_CAPACITY);
        let mut link = self.lock();
        link.epoch = epoch;
        link.notes = Some(tx);
        rx
    }

    /// Stops all audio from reaching the playback queue until the next turn
    /// begins. Returns after any enqueue that was already running.
    pub(crate) fn end_turn(&self) {
        let mut link = self.lock();
        link.epoch = 0;
        link.notes = None;
    }

    pub(crate) fn sink(self: &Arc<Self>) -> impl FnMut(TtsEvent) -> TtsSinkResult + Send + 'static {
        let speaker = Arc::clone(self);
        move |event| speaker.on_event(event)
    }

    fn note(&self, notes: &mpsc::Sender<TtsNote>, note: TtsNote) {
        if notes.try_send(note).is_err() {
            Counters::bump(&self.counters.notes_dropped);
            tracing::warn!("a speech note was dropped because the turn was not reading them");
        }
    }

    fn on_event(&self, event: TtsEvent) -> TtsSinkResult {
        match event {
            TtsEvent::Ready { info, .. } => {
                self.inbox.post(Inbound::Tts(TtsLifecycle::Ready(info)));
                TtsSinkResult::Accepted
            }
            TtsEvent::LoadFailed { error } => {
                self.inbox
                    .post(Inbound::Tts(TtsLifecycle::LoadFailed(error)));
                TtsSinkResult::Accepted
            }
            TtsEvent::Stopped => {
                self.inbox.post(Inbound::Tts(TtsLifecycle::Stopped));
                TtsSinkResult::Accepted
            }
            TtsEvent::Cancelled { turn_id, .. } => {
                let link = self.lock();
                if let (true, Some(notes)) = (link.epoch == turn_id, link.notes.as_ref()) {
                    self.note(notes, TtsNote::Cancelled);
                }
                TtsSinkResult::Accepted
            }
            TtsEvent::Failed { turn_id, error, .. } => {
                let link = self.lock();
                if let (true, Some(notes)) = (link.epoch == turn_id, link.notes.as_ref()) {
                    self.note(
                        notes,
                        TtsNote::Failed {
                            message: error.to_string(),
                        },
                    );
                }
                TtsSinkResult::Accepted
            }
            TtsEvent::Audio {
                turn_id,
                index,
                chunk,
                info,
                synth_ms,
                since_submit_ms,
            } => {
                let link = self.lock();
                let Some(notes) = link.notes.as_ref().filter(|_| link.epoch == turn_id) else {
                    Counters::bump(&self.counters.stale_audio);
                    return TtsSinkResult::Accepted;
                };
                // The instant is read just before the hand-over, so converting the
                // chunk to the device rate counts as output start, not as TTS time.
                let at = self.clock.now();
                match self.playback.enqueue(&chunk) {
                    Ok(EnqueueOutcome::Queued) => {
                        self.note(notes, TtsNote::Audio { at });
                        TtsSinkResult::Accepted
                    }
                    // A stop ran while the chunk was being converted.
                    Ok(EnqueueOutcome::DiscardedByStop) => {
                        Counters::bump(&self.counters.stale_audio);
                        TtsSinkResult::Accepted
                    }
                    Err(PlaybackError::QueueFull { .. }) => {
                        drop(link);
                        TtsSinkResult::Full(TtsEvent::Audio {
                            turn_id,
                            index,
                            chunk,
                            info,
                            synth_ms,
                            since_submit_ms,
                        })
                    }
                    Err(error) => {
                        self.note(
                            notes,
                            TtsNote::Failed {
                                message: error.to_string(),
                            },
                        );
                        TtsSinkResult::Accepted
                    }
                }
            }
        }
    }
}
