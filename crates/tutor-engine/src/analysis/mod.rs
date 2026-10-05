//! Turn analysis (contract T2): the prompt, the semantic filters and the
//! background service that stores the result.

mod filter;
mod prompt;
mod service;

pub use filter::{
    AnalysedTurn, DROP_WINDOW, DropWindow, ErrorFinding, EvidenceFinding, EvidenceStatus,
    FilterContext, Filtered, MAX_NOTE_WORDS, Notes, RawAnalysis, Understood, apply_filters,
};
pub use prompt::{
    AnalysisKind, AnalysisMessage, InputMode, MAX_ERRORS_DRAFT, MAX_ERRORS_TURN, ObjectiveRef,
    TURN_ANALYSIS_VERSION, TurnInput, system_prompt as analysis_system_prompt,
    user_message as analysis_user_message,
};
pub use service::{
    AnalysedRecord, AnalysisFailure, AnalyzerConfig, BATCH_EVERY, Cadence, MAX_BATCH_TURNS,
    MAX_HARD_FAILURES, MAX_QUEUE, RunReport, TurnAnalyzer, TurnToAnalyse, UNRELIABLE_SETTING,
    analysis_unreliable,
};
