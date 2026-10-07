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

mod chunker;
mod event;
mod llm;
mod prompt;
mod session;
mod turn;

pub use chunker::SentenceChunker;
pub use event::UiEvent;
pub use llm::{LlmClient, TextStream};
pub use prompt::{
    FALLBACK_LINE, FeedbackMode, Focus, HISTORY_MESSAGES, MAX_NOTES, OPENING_INSTRUCTION,
    PronFinding, ReplyLimits, ScenarioError, T1_TEMPERATURE, TUTOR_TURN_VERSION, TutorContext,
    bounded_history, opening_request, reply_limits, system_prompt, text_request, user_message,
};
pub use session::{
    Channel, EndReason, EngineFault, Event, Phase, Session, SessionKind, TransitionError, TurnState,
};
pub use turn::{ReplyOutcome, ReplyReport, run_reply};
