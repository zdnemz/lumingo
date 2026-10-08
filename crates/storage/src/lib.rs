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
//!
//! # Opening
//!
//! [`Database::open`] creates or opens the file in WAL mode with foreign keys on
//! and a busy timeout, copies the old file to `<db>.bak-<old version>` before a
//! migration that changes an existing schema (the newest two backups are kept),
//! and runs the migrations. One write connection serialises writes; a small pool
//! reads. A file written by a newer build, or whose applied migration was edited,
//! is refused and left untouched.
//!
//! # Repositories
//!
//! Each is a method on [`Database`] that returns a short-lived handle:
//! `profiles`, `settings`, `providers`, `models`, `curriculum`, `sessions`,
//! `turns`, `analysis`, `generated_content`, `audio_clips`, `attempts`,
//! `evidence`, `pending_scoring`, `estimates`, `pron_results`, `unit_progress`,
//! `mastery`, `error_stats`, `review_schedule`, `diagnostics` and `game`.
//!
//! They are plain inserts and reads. Scoring, eligibility and levels belong to
//! the assessment crate: `estimates` stores the level it is given and nothing
//! here derives one. The game layer (XP, streaks, cosmetics) shares no table or
//! key with attempts, evidence or estimates; see `migrations/0002_game.sql`.
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
mod progress;
mod pron;
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
pub use progress::{
    ErrorStat, ErrorStats, MasteryRepo, NewReviewItem, ObjectiveMastery, ReviewItem,
    ReviewSchedule, UnitProgress, UnitProgressRepo,
};
pub use pron::{NewPronResult, PronResult, PronResults};
pub use providers::{NewProviderProfile, ProviderProfile, Providers};
pub use sessions::{NewSession, Session, Sessions};
pub use settings::{Setting, Settings};
pub use streak::StreakStatus;
pub use time::{LocalDate, TimeError, Timestamp};
pub use turns::{NewTurn, Turn, Turns};
