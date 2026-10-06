//! Tutoring logic. Handlers in `apps/server` call into this crate and never
//! contain tutoring, scoring or prompt logic themselves.
#![forbid(unsafe_code)]

mod activity;
mod analysis;
mod chat;
mod chunker;
mod drafts;
mod error;
mod evidence;
mod practice;
mod prompt;
mod reading;
mod review;
mod rubric;
mod session;
mod support;
mod topics;
mod workshop;

pub use activity::*;
pub use analysis::*;
pub use chat::{
    ChatConfig, ChatDeps, ChatReply, ChatSummary, ChatTopic, ErrorPattern, MAX_TOPIC_CHARS,
    ReplyOutcome, TextChat, clean_topic,
};
pub use chunker::SentenceChunker;
pub use drafts::{DraftComparison, DraftError, EarlierError, Resolution, compare_drafts};
pub use error::{EngineError, Result};
pub use evidence::{
    CONFIDENCE_FLOOR, DeterministicRecord, EngineRole, EngineStamp, EvidenceRecorder,
    PRON_ALGORITHM_VERSION, PRON_CONFIDENCE_CAP, PronRecord, RECORDER_VERSION, Recorded,
    RubricMeta, Subject, counts_toward_estimate, rubric_scorer_version,
};
pub use practice::*;
pub use prompt::{
    FALLBACK_LINE, FeedbackMode, Focus, HISTORY_MESSAGES, MAX_NOTES, PronFinding, ReplyLimits,
    TUTOR_TURN_VERSION, TutorContext, bounded_history, reply_limits, system_prompt, user_message,
};
pub use reading::*;
pub use review::{Grade, ReviewState, due_order, update_mastery};
pub use rubric::{
    Alarm, CheckedRun, ConfidenceInputs, CrossCheck, CurriculumWords, Dimension, DimensionResult,
    DimensionStatus, MAX_BAND_GAP, MAX_FEEDBACK_WORDS, PointResult, RUBRIC_ALGORITHM_VERSION,
    RUBRIC_SCORE_VERSION, RawDimension, RawPoint, RawRubric, RubricDimension, RubricOutcome,
    RubricResult, ScoredDimension, WorkshopRubric, WorkshopTask, cross_check, merge_rerun,
    rubric_confidence, system_prompt as rubric_system_prompt, user_message as rubric_user_message,
};
pub use session::{
    Channel, EndReason, EngineFault, Event, Phase, Session, SessionKind, TransitionError, TurnState,
};
pub use support::{Clock, system_clock};
pub use topics::{
    ConversationTopic, LevelBank, LocalizedText, ReadingTopic, TopicBank, WritingPrompt,
};
pub use workshop::*;
