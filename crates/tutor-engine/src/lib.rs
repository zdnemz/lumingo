//! Tutoring logic: the session state machine, the events for the UI, the
//! streamed tutor reply and the T1 prompt (`context_pack.md` section 9,
//! `PROMPT_CONTRACTS.md` call type T1).
//!
//! The crate is pure logic plus one streamed call. It owns no engines, no
//! database and no HTTP: the caller applies learner-side events to the
//! [`Session`], builds a request from a [`TutorContext`], and drives one reply
//! with [`run_reply`]. Tests drive the same paths with a scripted
//! [`LlmClient`]; a fake engine never exists in a release build.
#![forbid(unsafe_code)]

mod analysis;
mod chat;
mod chunker;
mod event;
mod llm;
mod prompt;
mod session;
mod topics;
mod turn;
mod unit;

pub use analysis::{
    ActivityError, AnalysisCadence, AnalysisFailure, AnalysisInput, AnalysisOutcome, AnalysisTurn,
    BATCH_SIZE, CONVERSATION_ERROR_CAP, DRAFT_ERROR_CAP, DropCounts, ErrorFinding,
    FilteredAnalysis, InputMode, NotesForNextTurn, ObjectiveEvidence, ObjectivePair,
    ReliabilityWindow, TURN_ANALYSIS_VERSION, TurnAnalysisEntry, analysis_request,
    analysis_system_prompt, filter_output, run_analysis,
};
pub use chat::{
    CHAT_ATTEMPT_SCORER_VERSION, Chat, ChatAttempt, ChatConfig, ChatError, ChatSummary, ChatTopic,
    ChatTurn, ErrorPattern, MAX_TOPIC_CHARS, SummarisedTurn, clean_topic, session_summary,
};
pub use chunker::SentenceChunker;
pub use event::UiEvent;
pub use llm::{LlmClient, TextStream};
pub use prompt::{
    FALLBACK_LINE, FeedbackMode, Focus, HISTORY_MESSAGES, MAX_NOTES, OPENING_INSTRUCTION,
    PronFinding, ReplyLimits, ScenarioError, T1_TEMPERATURE, TUTOR_TURN_VERSION, TutorContext,
    activity_target_language, bounded_history, opening_request, reply_limits, system_prompt,
    text_request, user_message,
};
pub use session::{
    Channel, EndReason, EngineFault, Event, Phase, Session, SessionKind, TransitionError, TurnState,
};
pub use topics::{ConversationTopic, LevelBank, ReadingTopic, TopicBank, WritingPrompt};
pub use turn::{ReplyOutcome, ReplyReport, run_reply};
pub use unit::{
    CheckpointReport, CheckpointRow, PENDING_SCORER_VERSION, PRODUCTION_PENDING_VERSION,
    PendingProduction, StoredAttempt, Submission, SubmissionOutcome, SubmitError,
    checkpoint_report, submit,
};
