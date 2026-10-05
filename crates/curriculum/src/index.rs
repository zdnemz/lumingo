//! The plain row `app-core` writes to the database index.
//!
//! This crate does not know the storage layer. It only says what an index row holds, so
//! `app-core` can compare `checksum` with the stored value and re-index a unit when the
//! file changed.

use serde::{Deserialize, Serialize};

use crate::model::{Level, Localized, Skill, Unit};

/// Number of activities per skill in one unit. Both speaking skills count as `speaking`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct SkillCounts {
    pub listening: u32,
    pub speaking: u32,
    pub reading: u32,
    pub writing: u32,
    pub mediation: u32,
    pub grammar: u32,
    pub vocabulary: u32,
    pub pronunciation: u32,
}

impl SkillCounts {
    /// Counts the activities of `unit` by their `skill` field.
    pub fn of_unit(unit: &Unit) -> Self {
        let mut counts = SkillCounts::default();
        for activity in &unit.activities {
            let slot = match activity.skill() {
                Skill::Listening => &mut counts.listening,
                Skill::Reading => &mut counts.reading,
                Skill::SpeakingProduction | Skill::SpeakingInteraction => &mut counts.speaking,
                Skill::Writing => &mut counts.writing,
                Skill::Mediation => &mut counts.mediation,
                Skill::Grammar => &mut counts.grammar,
                Skill::Vocabulary => &mut counts.vocabulary,
                Skill::Pronunciation => &mut counts.pronunciation,
            };
            *slot += 1;
        }
        counts
    }
}

/// One row of the unit index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct UnitIndexEntry {
    pub id: String,
    pub level: Level,
    pub title: Localized,
    /// SHA-256 of the unit file's bytes, lower-case hex.
    pub checksum: String,
    pub skill_counts: SkillCounts,
    pub estimated_minutes: u32,
}

impl UnitIndexEntry {
    pub fn new(unit: &Unit, checksum: &str) -> Self {
        Self {
            id: unit.id.clone(),
            level: unit.level,
            title: unit.title.clone(),
            checksum: checksum.to_owned(),
            skill_counts: SkillCounts::of_unit(unit),
            estimated_minutes: unit.estimated_minutes,
        }
    }
}
