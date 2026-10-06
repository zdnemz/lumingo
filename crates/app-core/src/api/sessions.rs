//! Session and turn route groups: starting, stopping and talking.

use curriculum::Level;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::feedback::FeedbackView;
use super::mirror::api_enum;
use super::progress::{SessionKind, SessionStatus};

api_enum! {
    /// Whether the tutor corrects while the learner talks (PRD FR-C): fluency mode
    /// never interrupts, accuracy mode makes at most one correction per turn.
    FeedbackMode <=> tutor_engine::FeedbackMode {
        Fluency => "fluency",
        Accuracy => "accuracy",
    }
}

/// Where a session's conversation topic comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum TopicChoice {
    /// A topic the learner typed. It is cut to one line of at most 80 characters.
    Typed { text: String },
    /// An entry of the topic bank for the chosen level.
    Bank { id: String },
}

/// `POST /api/sessions`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StartSessionRequest {
    pub kind: SessionKind,
    /// The unit of a lesson, checkpoint, drill, or of a conversation that plays
    /// the unit's roleplay.
    #[serde(default)]
    pub unit_id: Option<String>,
    /// A roleplay activity of the unit, for a conversation.
    #[serde(default)]
    pub activity_id: Option<String>,
    /// Fluency or accuracy. Without one, the activity's own mode, or fluency.
    #[serde(default)]
    pub mode: Option<FeedbackMode>,
    /// The topic of a chat, a conversation outside a unit, a writing prompt or a
    /// reading text.
    #[serde(default)]
    pub topic: Option<TopicChoice>,
    /// The level the learner picked for a free mode. It is the learner's choice
    /// and never something a model produced. Without one, A1.
    #[serde(default)]
    pub level: Option<Level>,
    /// For a voice conversation: speak the replies (the default) or show them.
    #[serde(default)]
    pub speak: Option<bool>,
}

/// How a conversation reaches the learner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SessionChannel {
    Voice,
    Text,
}

/// Where a turn is. Voice and text use different members.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum TurnPhase {
    Listening,
    Transcribing,
    Thinking,
    Speaking,
    Waiting,
    Replying,
}

/// How a session stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SessionLife {
    Active,
    Paused,
    /// The provider failed twice. `resume` tries again.
    ProviderUnavailable,
    EngineError,
    Ended,
}

/// Which engine failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum EngineFaultView {
    Microphone,
    SpeechRecognition,
    SpeechSynthesis,
    Playback,
}

/// Who said a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum LineRole {
    Learner,
    Tutor,
}

/// One line of the conversation as the screen shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TurnLine {
    /// The number the events of this turn carry.
    pub turn: u64,
    pub role: LineRole,
    pub text: String,
    /// The position in the stored session, once the turn is stored.
    pub seq: Option<i64>,
    /// The learner corrected the transcript.
    pub edited: bool,
}

/// How many recent lines the snapshot holds, at most. Older lines are in the
/// stored session.
pub const RECENT_LINES: usize = 12;

/// The longest line the snapshot holds, in characters. A longer line is cut.
pub const RECENT_LINE_CHARS: usize = 1_000;

/// The running session, as the snapshot and the start and stop answers show it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ActiveSessionView {
    pub id: i64,
    pub kind: SessionKind,
    pub unit_id: Option<String>,
    pub channel: SessionChannel,
    pub life: SessionLife,
    /// Set while `life` is `active`.
    pub turn_state: Option<TurnPhase>,
    /// Set while `life` is `engine_error`.
    pub fault: Option<EngineFaultView>,
    /// Turns that ran to the end.
    pub turns_completed: u32,
    /// The number of the turn that is running or was the last one.
    pub turn: u64,
    /// The newest lines, oldest first, at most [`RECENT_LINES`].
    pub recent: Vec<TurnLine>,
    /// The tutor's reply so far, while it is being written. It lets a page that
    /// was reloaded mid-reply show what has arrived.
    pub pending_reply: Option<String>,
    /// Replies are spoken (a voice conversation with speech output).
    pub speaking: bool,
    /// The activity the learner is on, for a lesson, checkpoint or drill.
    pub activity_id: Option<String>,
}

/// What a stopped session leaves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SessionEnded {
    pub id: i64,
    pub kind: SessionKind,
    pub status: SessionStatus,
    /// The summary: what to practise next, or the checkpoint result. `None` for a
    /// cancelled session.
    pub feedback: Option<FeedbackView>,
}

/// `POST /api/sessions/{id}/stop`. The body may be empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StopRequest {
    /// Cancel instead of finishing: the stored session is marked aborted, no more
    /// analysis is started and no summary is made.
    #[serde(default)]
    pub cancel: bool,
}

/// `POST /api/sessions/{id}/text`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SendTextRequest {
    pub text: String,
}

/// The answer to a turn that was accepted. The reply arrives as events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TurnAccepted {
    pub session_id: i64,
    /// The number the events of this turn carry.
    pub turn: u64,
}

/// `POST /api/sessions/{id}/turns/{seq}/edit`: the learner's corrected words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EditTurnRequest {
    pub text: String,
}

/// Whether the analysis of an edited turn is of the edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum EditEffect {
    /// The turn was not stored yet: the model and the analysis see the edit.
    BeforeRecording,
    /// The turn was stored and its analysis had not started: it uses the edit.
    AnalysedFromEdit,
    /// The analysis had started or finished: it is of the words the recogniser
    /// gave. The corrected text is stored and the original is kept next to it.
    StoredOnly,
}

/// The answer to an edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EditResult {
    pub session_id: i64,
    pub turn: u64,
    pub effect: EditEffect,
}

/// `POST /api/sessions/{id}/push-to-talk`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PushToTalkRequest {
    /// True when the learner presses the key, false when they let go.
    pub pressed: bool,
}

/// What a command that changes nothing the learner reads answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Ack {
    pub ok: bool,
}
