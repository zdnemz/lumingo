//! The WebSocket event stream.
//!
//! Every event carries a sequence number that grows by one per published event.
//! The first message of a connection is a `Snapshot`; a client that falls behind
//! is sent a new `Snapshot` and replaces its state with it. There is no replay.
//!
//! # Order of a text chat turn
//!
//! 1. `TranscriptFinal` (the learner's line, with its stored position)
//! 2. `TurnState` `thinking`
//! 3. `TurnState` `replying` when the first words arrive, and one `TutorTextDelta`
//!    per piece of the reply
//! 4. `TurnState` `waiting`, carrying `tutor_turn_seq` (the reply is stored)
//! 5. `AnalysisReady` for the learner's line, whenever the background analysis
//!    finishes, never before step 4
//!
//! `FeedbackReady` follows the end of a unit activity, a writing draft, a reading
//! score and the end of a session. A voice turn runs `TurnState` `listening`,
//! `transcribing`, `thinking`, `speaking`, `listening`, with `TranscriptFinal`,
//! `TutorSentenceSpoken` and `LatencyReport` between.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::common::ErrorCode;
use super::engines::EngineView;
use super::feedback::{FeedbackView, PronFindingsView, TurnAnalysisView};
use super::models::DownloadState;
use super::progress::SessionKind;
use super::providers::ProviderInfo;
use super::sessions::{EngineFaultView, SessionChannel, SessionLife, TurnPhase};
use super::state::StateSnapshot;

/// One message on the event stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
#[ts(export)]
pub enum ServerEvent {
    /// Always the first message on a connection, and sent again to a client that
    /// fell behind. A client replaces its state with it.
    Snapshot { seq: u64, state: Box<StateSnapshot> },
    /// Periodic proof that the stream is alive.
    Heartbeat { seq: u64, uptime_ms: u64 },
    /// The active provider changed, or a connection test finished. `provider` is
    /// the active profile after the change, `None` when there is none.
    ProviderStatus {
        seq: u64,
        provider: Option<ProviderInfo>,
    },
    /// The session started, paused, resumed, lost its provider, hit an engine
    /// fault or ended.
    SessionState {
        seq: u64,
        session_id: i64,
        kind: SessionKind,
        life: SessionLife,
        fault: Option<EngineFaultView>,
        /// A sentence for the learner when the state needs one.
        message: Option<String>,
    },
    /// The turn moved to another phase. `tutor_turn_seq` is set on the move that
    /// ends a reply: the reply is stored at that position and can be spoken again.
    TurnState {
        seq: u64,
        session_id: i64,
        turn: u64,
        state: TurnPhase,
        tutor_turn_seq: Option<i64>,
    },
    /// The microphone level, at most ten per second.
    MicLevel { seq: u64, level: f32 },
    /// The learner's line of a turn, final. A spoken turn sends it twice: when it
    /// was heard (`turn_seq` is `None`) and when it was stored. A client keys it
    /// by `turn`.
    TranscriptFinal {
        seq: u64,
        session_id: i64,
        turn: u64,
        text: String,
        source: SessionChannel,
        turn_seq: Option<i64>,
        edited: bool,
    },
    /// A piece of the tutor's reply in a text conversation.
    TutorTextDelta {
        seq: u64,
        session_id: i64,
        turn: u64,
        delta: String,
    },
    /// A sentence of the tutor's reply, as it is handed to speech output.
    TutorSentenceSpoken {
        seq: u64,
        session_id: i64,
        turn: u64,
        index: u32,
        text: String,
    },
    /// Pronunciation findings of a recording. Always experimental.
    PronFindings {
        seq: u64,
        session_id: i64,
        activity_id: String,
        findings: PronFindingsView,
    },
    /// The background analysis of a learner turn is stored.
    AnalysisReady {
        seq: u64,
        session_id: i64,
        analysis: TurnAnalysisView,
    },
    /// Feedback on a piece of work is ready.
    FeedbackReady {
        seq: u64,
        session_id: i64,
        feedback: FeedbackView,
    },
    /// The five latency parts of a voice turn, in milliseconds. A part is `None`
    /// when it was not on the turn's path.
    LatencyReport {
        seq: u64,
        session_id: i64,
        turn: u64,
        endpointing_wait_ms: Option<f64>,
        stt_finalise_ms: Option<f64>,
        llm_first_sentence_ms: Option<f64>,
        tts_first_sentence_ms: Option<f64>,
        output_start_ms: Option<f64>,
        sum_ms: f64,
        complete: bool,
    },
    /// An engine changed state: loaded, failed, or found missing.
    EngineStatus { seq: u64, engine: EngineView },
    /// A model download made progress or ended.
    DownloadProgress {
        seq: u64,
        model_id: String,
        file: String,
        file_index: u32,
        file_count: u32,
        bytes_done: u64,
        bytes_total: Option<u64>,
        state: DownloadState,
        /// For `failed`: why, in a sentence without a path.
        message: Option<String>,
    },
    /// Something went wrong outside a request: a turn that failed, an analysis
    /// that could not be stored.
    Error {
        seq: u64,
        session_id: Option<i64>,
        code: ErrorCode,
        message: String,
    },
}

impl ServerEvent {
    /// The sequence number of the event.
    pub fn seq(&self) -> u64 {
        match self {
            Self::Snapshot { seq, .. }
            | Self::Heartbeat { seq, .. }
            | Self::ProviderStatus { seq, .. }
            | Self::SessionState { seq, .. }
            | Self::TurnState { seq, .. }
            | Self::MicLevel { seq, .. }
            | Self::TranscriptFinal { seq, .. }
            | Self::TutorTextDelta { seq, .. }
            | Self::TutorSentenceSpoken { seq, .. }
            | Self::PronFindings { seq, .. }
            | Self::AnalysisReady { seq, .. }
            | Self::FeedbackReady { seq, .. }
            | Self::LatencyReport { seq, .. }
            | Self::EngineStatus { seq, .. }
            | Self::DownloadProgress { seq, .. }
            | Self::Error { seq, .. } => *seq,
        }
    }

    /// The name of the event, as it is in the `type` field.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Snapshot { .. } => "Snapshot",
            Self::Heartbeat { .. } => "Heartbeat",
            Self::ProviderStatus { .. } => "ProviderStatus",
            Self::SessionState { .. } => "SessionState",
            Self::TurnState { .. } => "TurnState",
            Self::MicLevel { .. } => "MicLevel",
            Self::TranscriptFinal { .. } => "TranscriptFinal",
            Self::TutorTextDelta { .. } => "TutorTextDelta",
            Self::TutorSentenceSpoken { .. } => "TutorSentenceSpoken",
            Self::PronFindings { .. } => "PronFindings",
            Self::AnalysisReady { .. } => "AnalysisReady",
            Self::FeedbackReady { .. } => "FeedbackReady",
            Self::LatencyReport { .. } => "LatencyReport",
            Self::EngineStatus { .. } => "EngineStatus",
            Self::DownloadProgress { .. } => "DownloadProgress",
            Self::Error { .. } => "Error",
        }
    }
}
