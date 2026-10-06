//! The activity runtime (S4-07, S4-08): present an authored activity, accept a
//! response, score it, give feedback data back and leave rows behind.
//!
//! Deterministic activities (`mcq`, `gap_fill`, `reorder`, `match`, `dictation`,
//! `reading_set`, `listening_set`, `error_correction`, `minimal_pairs` in listen
//! mode) are scored here with the scorers of `assessment-engine` and normaliser
//! `norm/1`. Productive activities and drills go through the rubric scorer and
//! the pronunciation seam. Everything that is stored goes through
//! [`crate::EvidenceRecorder`].
//!
//! Nothing in this module has a channel. The bounded state it keeps is the play
//! count of each audio item (a `u8` that saturates at its limit), the number of
//! learner turns of a roleplay (at most the activity's `max_turns`), and the
//! sizes of responses, which are refused when over [`MAX_TEXT_CHARS`],
//! [`MAX_CLIP_SAMPLES`], [`MAX_TOTAL_SAMPLES`] or [`MAX_CLIPS`].

mod drill;
mod drills;
mod error;
mod feedback;
mod player;
mod present;
mod productive;
mod replay;
mod response;
mod roleplay;
mod score;

pub use drill::{DrillError, DrillReport, DrillRequest, DrillScorer, PronEngineDrill};
pub use error::ActivityError;
pub use feedback::{Feedback, ItemFeedback, ItemOutcome};
pub use player::{
    ActivityResult, AudioPlay, CheckpointReport, CheckpointRow, ResultOutcome, UnitConfig, UnitEnv,
    UnitPlayer, UnitSummary, UnscoredReason, checkpoint_report, ensure_indexed, settle_unit,
};
pub use present::{
    AudioLine, Body, PairItem, PairView, Presentation, QuestionView, audio_of, match_display_order,
    minimal_pair_spoken, present,
};
pub use replay::PlayCounter;
pub use response::{
    Clip, MAX_CLIP_SAMPLES, MAX_CLIPS, MAX_TEXT_CHARS, MAX_TOTAL_SAMPLES, Response, SpokenResponse,
    check_clips, check_text,
};
pub use roleplay::RoleplayRun;
pub use score::{Scored, evidence_skill, score_deterministic};
