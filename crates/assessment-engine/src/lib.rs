//! Scoring a response and estimating a level are two separate steps.
//!
//! This crate holds the pure parts of both: answer normalisation and
//! deterministic scoring, and the level estimator. It never calls a language
//! model, and no model output can set or change a level.
#![forbid(unsafe_code)]

mod deterministic;
mod estimate;
mod normalise;
mod types;

pub use deterministic::{
    GapScore, SUCCESS_THRESHOLD, is_success, score_dictation, score_error_correction,
    score_gap_fill, score_match, score_mcq, score_reorder, score_share,
};
pub use estimate::{
    ALGORITHM_VERSION, Confidence, EstimateStatus, LevelEvidence, Profile, SkillEstimate,
    confidence_band, estimate_profile, estimate_skill, wilson_lower_bound,
};
pub use normalise::{NORM_VERSION, edit_distance, normalize};
pub use types::{Attempt, AttemptStatus, Level, Origin, Scorer, Skill};
