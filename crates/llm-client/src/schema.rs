//! The four structured-output contracts. The schema files live in `contracts/`
//! at the repository root and are embedded at compile time, so the binary
//! validates against exactly the files that are reviewed in the repository.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Contract {
    TurnAnalysis,
    RubricScore,
    PracticeItems,
    ReadingPassage,
}

const TURN_ANALYSIS: &str = include_str!("../../../contracts/turn_analysis.schema.json");
const RUBRIC_SCORE: &str = include_str!("../../../contracts/rubric_score.schema.json");
const PRACTICE_ITEMS: &str = include_str!("../../../contracts/practice_items.schema.json");
const READING_PASSAGE: &str = include_str!("../../../contracts/reading_passage.schema.json");

impl Contract {
    pub const ALL: [Contract; 4] = [
        Self::TurnAnalysis,
        Self::RubricScore,
        Self::PracticeItems,
        Self::ReadingPassage,
    ];

    /// The identifier used in `capabilities.contracts_ok` and as the schema name
    /// sent to providers. It matches `^[a-zA-Z0-9_-]{1,64}$`, which OpenAI requires.
    pub fn name(self) -> &'static str {
        match self {
            Self::TurnAnalysis => "turn_analysis",
            Self::RubricScore => "rubric_score",
            Self::PracticeItems => "practice_items",
            Self::ReadingPassage => "reading_passage",
        }
    }

    pub(crate) fn raw(self) -> &'static str {
        match self {
            Self::TurnAnalysis => TURN_ANALYSIS,
            Self::RubricScore => RUBRIC_SCORE,
            Self::PracticeItems => PRACTICE_ITEMS,
            Self::ReadingPassage => READING_PASSAGE,
        }
    }

    /// The schema document as committed in `contracts/`. The bytes are embedded
    /// at compile time and a unit test parses all four, so the fallback to
    /// `Value::Null` is unreachable in a build that passes its tests.
    pub fn schema(self) -> &'static Value {
        static CELLS: [OnceLock<Value>; 4] = [
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
        ];
        let index = Self::ALL.iter().position(|c| *c == self).unwrap_or(0);
        CELLS[index].get_or_init(|| serde_json::from_str(self.raw()).unwrap_or(Value::Null))
    }

    /// The schema as sent to a provider: the documentation keywords of the root
    /// (`$schema`, `$id`, `title`, `description`) are removed because some
    /// strict-mode servers reject keywords they do not list. Validation still
    /// uses the full document.
    pub fn wire_schema(self) -> Value {
        strip_root_annotations(self.schema())
    }
}

pub(crate) fn strip_root_annotations(schema: &Value) -> Value {
    let mut wire = schema.clone();
    if let Some(object) = wire.as_object_mut() {
        for key in ["$schema", "$id", "title", "description"] {
            object.remove(key);
        }
    }
    wire
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_four_contracts_parse_and_are_objects() {
        for contract in Contract::ALL {
            let schema = contract.schema();
            assert!(schema.is_object(), "{} did not parse", contract.name());
            assert_eq!(schema["type"], "object");
        }
    }

    #[test]
    fn wire_schema_drops_root_annotations_only() {
        let wire = Contract::TurnAnalysis.wire_schema();
        let object = wire.as_object().expect("object");
        assert!(!object.contains_key("$schema"));
        assert!(!object.contains_key("$id"));
        assert!(!object.contains_key("title"));
        assert!(!object.contains_key("description"));
        assert!(object.contains_key("properties"));
        assert_eq!(object["additionalProperties"], false);
    }

    #[test]
    fn names_are_valid_openai_schema_names() {
        for contract in Contract::ALL {
            let name = contract.name();
            assert!(name.len() <= 64);
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            );
        }
    }
}
