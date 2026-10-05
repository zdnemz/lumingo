//! Local database for Lumingo: migrations and the repositories that wrap them.
//!
//! The schema lives in `migrations/`. Once a migration has shipped, the file in
//! this crate is the source of truth and must never be edited; a change is a new
//! numbered file.
//!
//! Other crates never see SQL. They open a [`Database`] and call repository
//! functions with plain serde types. Every timestamp and every calendar day is
//! supplied by the caller, so the crate has no hidden dependence on the clock or
//! the time zone.
#![forbid(unsafe_code)]

mod analysis;
mod attempts;
mod content;
mod curriculum;
mod db;
mod diagnostics;
mod enums;
mod error;
mod estimates;
mod game;
mod models;
mod profiles;
mod providers;
mod row;
mod sessions;
mod settings;
mod streak;
mod time;
mod turns;

pub use analysis::{Analysis, ErrorEvent, NewErrorEvent, TurnAnalysis};
pub use attempts::{
    Attempt, Attempts, Evidence, EvidenceRepo, NewAttempt, NewEvidence, PendingScoring,
    PendingScoringRepo, ResponseGroup, ScoreUpdate, group_by_response,
};
pub use content::{
    AudioClip, AudioClips, GeneratedContent, GeneratedContentRepo, NewGeneratedContent,
};
pub use curriculum::{
    Curriculum, CurriculumVersion, IndexStatus, IndexedObjective, IndexedUnit,
    NewCurriculumVersion, NewObjective, NewUnit,
};
pub use db::{Database, OpenConfig, SCHEMA_VERSION};
pub use diagnostics::{Diagnostics, LlmCall, NewLlmCall, NewPerfSample, PerfSample};
pub use enums::*;
pub use error::{Result, StorageError};
pub use estimates::{Estimates, NewSkillEstimate, SkillEstimate};
pub use game::{
    ActivityRecorded, EquippedCosmetics, Game, MAX_XP_AWARD, NewXp, RestToken, StreakDay, Unlocked,
    XpAward, XpBySource, XpEntry, XpTotals,
};
pub use models::{InstalledModel, Models};
pub use profiles::{NewProfile, Profile, Profiles};
pub use providers::{NewProviderProfile, ProviderProfile, Providers};
pub use sessions::{NewSession, Session, Sessions};
pub use settings::{Setting, Settings};
pub use streak::StreakStatus;
pub use time::{LocalDate, TimeError, Timestamp};
pub use turns::{NewTurn, Turn, Turns};
