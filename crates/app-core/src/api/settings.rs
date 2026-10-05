//! Settings: the learner profile fields and the few switches the product
//! requirements name. Provider profiles are not settings; they have their own
//! group because they hold keys.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::common::{L1HelpMode, UiLanguage};

/// How the tutor reply and the phoneme analysis are ordered (PRD FR-P3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AdaptiveTiming {
    /// Wait for the analysis only on machines that measure fast enough.
    #[default]
    Auto,
    AlwaysWait,
    NeverWait,
}

/// Everything the settings screen edits. `PUT /api/settings` replaces all of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Settings {
    /// What the tutor calls the learner. 1 to 64 characters.
    pub display_name: String,
    pub ui_language: UiLanguage,
    /// The learner's first language as a two-letter ISO 639-1 code, such as `id`.
    pub l1: String,
    pub l1_help_mode: L1HelpMode,
    pub adaptive_timing: AdaptiveTiming,
    /// Opt-in: keep recent recordings for replay (PRD FR-D2). Off by default.
    pub keep_recordings: bool,
}
