//! Tutoring logic: the session state machine, the events for the UI and the
//! streamed tutor reply (`context_pack.md` section 9).
//!
//! The crate is pure logic plus one streamed call. It owns no engines, no
//! database and no HTTP: the caller applies learner-side events to the
//! [`Session`] and drives one reply with [`run_reply`]. Tests drive the same
//! paths with a scripted [`LlmClient`]; a fake engine never exists in a
//! release build.
#![forbid(unsafe_code)]

mod chunker;
mod event;
mod llm;
mod session;
mod turn;

pub use chunker::SentenceChunker;
pub use event::UiEvent;
pub use llm::{LlmClient, TextStream};
pub use session::{
    Channel, EndReason, EngineFault, Event, Phase, Session, SessionKind, TransitionError, TurnState,
};
pub use turn::{ReplyOutcome, ReplyReport, run_reply};
