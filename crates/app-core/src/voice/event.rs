//! What the voice loop tells the program around it.
//!
//! Events go out on a Tokio broadcast channel (capacity
//! [`VoiceConfig::event_capacity`](super::VoiceConfig::event_capacity), 64 by
//! default). Publishing never waits and never fails because nobody listens. A
//! receiver that falls behind loses the oldest events and learns how many
//! through `RecvError::Lagged`.
//!
//! Events carry the learner's words and the tutor's sentences because the
//! screen shows them. They are for display only: nothing in this crate writes
//! them to a log.

use tokio::sync::broadcast;
use tutor_engine::{EngineFault, Phase};

use super::latency::TurnLatency;

/// What made the tutor stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopCause {
    /// The learner pressed stop.
    Command,
    /// The learner held push-to-talk.
    PushToTalk,
    /// The learner started speaking while the tutor was still thinking.
    Speech,
}

/// How a turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnOutcome {
    /// The model's reply was spoken (or shown) in full.
    Replied,
    /// The reply was empty or refused, or nothing was heard, so the authored
    /// line was spoken instead.
    Fallback,
    /// The learner stopped the turn. What had arrived is kept.
    Stopped,
    /// The provider failed twice. The session waits for `resume`.
    ProviderUnavailable,
    /// A speech engine or the output failed. The session carries on with the
    /// next turn.
    EngineError,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VoiceEvent {
    /// The session phase changed.
    State(Phase),
    /// The endpointer confirmed that the learner started to speak.
    SpeechStarted,
    /// The tutor was stopped before it finished.
    Stopped(StopCause),
    /// The recogniser's transcript, or the typed message.
    Heard {
        turn: u64,
        text: String,
    },
    /// A sentence of the tutor's reply, as it is handed to speech output.
    TutorSentence {
        turn: u64,
        index: u32,
        text: String,
    },
    /// The latency parts of a turn, once its first tutor sample has played (or,
    /// without speech output, once its first sentence is ready).
    Latency(TurnLatency),
    TurnEnded {
        turn: u64,
        outcome: TurnOutcome,
    },
    /// The provider could not be reached or kept failing. `message` is for the learner.
    ProviderUnavailable {
        message: String,
    },
    /// A speech engine or the audio output failed. `message` is for the learner.
    EngineError {
        fault: EngineFault,
        message: String,
    },
    /// The session ended and nothing is left running. The last event.
    Closed,
}

/// Where events go. Cloning shares the channel.
#[derive(Debug, Clone)]
pub(crate) struct EventSink {
    tx: broadcast::Sender<VoiceEvent>,
}

impl EventSink {
    pub(crate) fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    pub(crate) fn publish(&self, event: VoiceEvent) {
        // An error only means that no one is subscribed.
        let _ = self.tx.send(event);
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<VoiceEvent> {
        self.tx.subscribe()
    }
}
