//! The per-level syllabus file, `curriculum/syllabus/<level>.json`.
//!
//! `docs/CURRICULUM_SPEC.md` section 7 names its fields (title, theme, objectives, grammar ids,
//! vocabulary field, functions, pronunciation focus) but ships no schema. The shape here is
//! inferred from that sentence and from rules X03 and X05, and is defined in
//! `curriculum/schema/syllabus.schema.json`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::load::{LoadError, parse_json};
use crate::model::{Level, Localized, Skill};
use crate::schema::SchemaChecker;

/// The syllabus schema, embedded like the unit schema.
pub const SYLLABUS_SCHEMA_JSON: &str =
    include_str!("../../../curriculum/schema/syllabus.schema.json");

/// What the syllabus plans for one objective of a unit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyllabusObjective {
    pub skill: Skill,
    pub can_do: Localized,
}

/// The plan for one unit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyllabusEntry {
    pub id: String,
    pub sequence: u8,
    pub title: Localized,
    pub theme: String,
    pub objectives: Vec<SyllabusObjective>,
    pub grammar_ids: Vec<String>,
    pub vocabulary_field: String,
    pub functions: Vec<String>,
    pub pronunciation_focus: Vec<String>,
}

/// The syllabus of one level: 30 entries once it is complete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Syllabus {
    pub schema_version: String,
    pub level: Level,
    pub units: Vec<SyllabusEntry>,
}

impl Syllabus {
    /// The entry planned for the unit with this id.
    pub fn entry(&self, unit_id: &str) -> Option<&SyllabusEntry> {
        self.units.iter().find(|entry| entry.id == unit_id)
    }
}

/// Checks `value` against the syllabus schema and converts it.
pub fn syllabus_from_value(value: &Value) -> Result<Syllabus, LoadError> {
    let checker = SchemaChecker::from_json("syllabus.schema.json", SYLLABUS_SCHEMA_JSON)?;
    let issues = checker.issues(value);
    if !issues.is_empty() {
        return Err(LoadError::Schema { issues });
    }
    serde_json::from_value(value.clone()).map_err(|err| LoadError::Model(err.to_string()))
}

/// Loads a syllabus from the bytes of a file.
pub fn load_syllabus_bytes(bytes: &[u8]) -> Result<Syllabus, LoadError> {
    syllabus_from_value(&parse_json(bytes)?)
}
