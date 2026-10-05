//! The cosmetic game layer: sparks, ranks and unlockables.
//!
//! Everything here is a reward for showing up. None of it is evidence of
//! language ability: this crate has no dependency on the assessment code, and
//! nothing it produces can be read as a CEFR level. A level estimate is computed
//! from stored attempts by the assessment engine and never from sparks.
#![forbid(unsafe_code)]

mod ranks;
mod sparks;
mod unlocks;

pub use ranks::{RANK_THRESHOLDS, RankInfo, rank_for};
pub use sparks::{SparkSource, award};
pub use unlocks::{Cosmetic, CosmeticKind, UNLOCKS, unlocked_at_rank, unlocked_for_sparks};
