//! Curriculum route group types.

use curriculum::{Level, Localized, SkillCounts, Unit};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One unit in the list: enough to draw a map, without the content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UnitSummary {
    pub id: String,
    pub level: Level,
    /// Position inside the level, 1 to 30.
    pub sequence: u8,
    pub title: Localized,
    pub theme: String,
    pub estimated_minutes: u32,
    pub skill_counts: SkillCounts,
    pub objective_count: u32,
    pub prerequisites: Vec<String>,
}

/// A unit file that was skipped, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UnitIssue {
    /// The file name relative to the units folder, or the folder itself.
    pub file: String,
    pub message: String,
}

/// `GET /api/units`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UnitList {
    /// Names the set of unit files that was indexed. `None` when no unit file
    /// could be read.
    pub content_version: Option<String>,
    /// In learning order: level, then sequence.
    pub units: Vec<UnitSummary>,
    /// Files that failed the schema or clashed with another unit. A bad file
    /// never hides the good ones.
    pub issues: Vec<UnitIssue>,
}

/// `GET /api/units/{id}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UnitDetail {
    pub unit: Unit,
}
