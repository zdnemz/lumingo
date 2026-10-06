//! The messages that reach the orchestrator, and the counters of what was lost.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use speech::{EngineInfo, SttEvent, TtsError};
use tokio::sync::{mpsc, oneshot};

use super::turn::TurnReport;

/// One finished utterance with the two instants the latency starts from.
#[derive(Debug, Clone)]
pub(crate) struct UtteranceMsg {
    /// 16 kHz mono samples.
    pub samples: Vec<f32>,
    /// The last sample the VAD classified as speech.
    pub last_speech: Duration,
    /// When the endpointer ended the utterance.
    pub ended: Duration,
}

#[derive(Debug)]
pub(crate) enum ListenEvent {
    SpeechStarted,
    PushToTalkStarted,
    Utterance(UtteranceMsg),
    /// The VAD failed. The message carries no audio and no text.
    Failed(String),
}

#[derive(Debug)]
pub(crate) enum Command {
    /// A typed message.
    Text(String),
    /// The tutor speaks first.
    Open,
    /// Stop the tutor and go back to listening.
    Stop,
    Pause,
    Resume,
    /// End the session. The sender is told when the orchestrator is done.
    Finish {
        cancelled: bool,
        done: oneshot::Sender<()>,
    },
}

#[derive(Debug)]
pub(crate) enum TtsLifecycle {
    Ready(EngineInfo),
    LoadFailed(TtsError),
    Stopped,
}

#[derive(Debug)]
pub(crate) enum Inbound {
    Listen(ListenEvent),
    Stt(SttEvent),
    Tts(TtsLifecycle),
    Command(Command),
    Turn(TurnReport),
}

/// Things that were dropped because a queue was full or a turn was over.
#[derive(Debug, Default)]
pub(crate) struct Counters {
    pub frames_dropped: AtomicU64,
    pub inbox_dropped: AtomicU64,
    pub utterances_dropped: AtomicU64,
    pub notes_dropped: AtomicU64,
    pub stale_audio: AtomicU64,
    pub recorder_dropped: AtomicU64,
    pub ptt_dropped: AtomicU64,
}

impl Counters {
    pub(crate) fn bump(counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn get(counter: &AtomicU64) -> u64 {
        counter.load(Ordering::Relaxed)
    }
}

/// The orchestrator's inbox. Cloning shares it.
#[derive(Clone)]
pub(crate) struct Inbox {
    tx: mpsc::Sender<Inbound>,
    counters: Arc<Counters>,
}

impl Inbox {
    pub(crate) fn new(tx: mpsc::Sender<Inbound>, counters: Arc<Counters>) -> Self {
        Self { tx, counters }
    }

    /// For threads that must not wait: a full inbox drops the message and counts it.
    pub(crate) fn post(&self, message: Inbound) {
        if self.tx.try_send(message).is_err() {
            Counters::bump(&self.counters.inbox_dropped);
            tracing::warn!("the voice inbox was full or closed; a message was dropped");
        }
    }

    /// For async callers: waits for room. False when the loop has stopped.
    pub(crate) async fn send(&self, message: Inbound) -> bool {
        self.tx.send(message).await.is_ok()
    }
}
