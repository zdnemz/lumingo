//! Local validation of structured output (PRD NFR-R4, `docs/PROMPT_CONTRACTS.md`
//! section 5): find the first complete JSON value in a reply, normalise the case
//! of enum strings, and validate against the contract schema.

use std::sync::OnceLock;

use jsonschema::Validator;
use serde_json::{Value, json};

use crate::error::InvalidReason;
use crate::schema::{Contract, strip_root_annotations};

/// Most violations kept per reply. A broken reply can have hundreds.
const MAX_VIOLATIONS: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Violation {
    /// JSON pointer into the reply, `""` for the root.
    pub path: String,
    /// The validator's message. It can quote the model's text, so it is sent back
    /// to the model in the repair call and never stored in an error value.
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Invalid {
    pub reason: InvalidReason,
    pub violations: Vec<Violation>,
}

/// A schema ready for use: the document as sent to the provider, the full
/// document for case-insensitive enum matching, and the compiled validator.
pub(crate) struct CompiledSchema {
    pub name: &'static str,
    pub wire: Value,
    full: Value,
    validator: Option<Validator>,
}

impl CompiledSchema {
    fn compile(name: &'static str, full: Value) -> Self {
        // `jsonschema` is built without its HTTP and file resolvers, so a `$ref` to
        // another document fails to compile instead of fetching it. The contracts
        // have no `$ref`. A compile failure is a bug that a unit test catches; at
        // run time it makes every reply invalid rather than accepting unchecked output.
        let validator = jsonschema::validator_for(&full).ok();
        Self {
            name,
            wire: strip_root_annotations(&full),
            full,
            validator,
        }
    }

    #[cfg(test)]
    pub(crate) fn is_compiled(&self) -> bool {
        self.validator.is_some()
    }

    /// Parses the reply and validates it. On success returns the normalised value.
    pub(crate) fn check_text(&self, text: &str) -> Result<Value, Invalid> {
        let Some(mut value) = extract_json(text) else {
            return Err(Invalid {
                reason: InvalidReason::NotJson,
                violations: Vec::new(),
            });
        };
        normalise_enum_case(&self.full, &mut value);
        match self.check_value(&value) {
            Ok(()) => Ok(value),
            Err(violations) => Err(Invalid {
                reason: InvalidReason::SchemaMismatch,
                violations,
            }),
        }
    }

    pub(crate) fn check_value(&self, value: &Value) -> Result<(), Vec<Violation>> {
        let Some(validator) = &self.validator else {
            return Err(vec![Violation {
                path: String::new(),
                message: "the schema could not be compiled".to_owned(),
            }]);
        };
        let violations: Vec<Violation> = validator
            .iter_errors(value)
            .take(MAX_VIOLATIONS)
            .map(|error| Violation {
                path: error.instance_path().to_string(),
                message: error.to_string(),
            })
            .collect();
        if violations.is_empty() {
            Ok(())
        } else {
            Err(violations)
        }
    }
}

/// The compiled schema of a contract, built once.
pub(crate) fn contract_schema(contract: Contract) -> &'static CompiledSchema {
    static CELLS: [OnceLock<CompiledSchema>; 4] = [
        OnceLock::new(),
        OnceLock::new(),
        OnceLock::new(),
        OnceLock::new(),
    ];
    let index = Contract::ALL
        .iter()
        .position(|c| *c == contract)
        .unwrap_or(0);
    CELLS[index].get_or_init(|| CompiledSchema::compile(contract.name(), contract.schema().clone()))
}

/// The three-field schema of probe step 3. It uses the same portable subset as
/// the contracts: every property required, `additionalProperties: false`, no
/// length or numeric constraints.
pub(crate) fn probe_schema() -> &'static CompiledSchema {
    static CELL: OnceLock<CompiledSchema> = OnceLock::new();
    CELL.get_or_init(|| {
        CompiledSchema::compile(
            "probe_test",
            json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["title", "word_count", "is_ok"],
                "properties": {
                    "title": { "type": "string" },
                    "word_count": { "type": "integer" },
                    "is_ok": { "type": "boolean" }
                }
            }),
        )
    })
}

/// Returns the first complete JSON object or array in `text`. Code fences and prose
/// around it are skipped, because that is what models add when they are only asked
/// to "return JSON".
pub(crate) fn extract_json(text: &str) -> Option<Value> {
    let trimmed = text.trim();
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        if value.is_object() || value.is_array() {
            return Some(value);
        }
    }
    for (start, ch) in trimmed.char_indices() {
        if ch != '{' && ch != '[' {
            continue;
        }
        let mut values = serde_json::Deserializer::from_str(&trimmed[start..]).into_iter::<Value>();
        if let Some(Ok(value)) = values.next() {
            return Some(value);
        }
    }
    None
}

/// Providers can return enum values with different capitalisation
/// (`docs/PROMPT_CONTRACTS.md` 3.2). Where a string is not an exact enum member but
/// equals one ignoring case, it is replaced by the member. Nothing else changes.
pub(crate) fn normalise_enum_case(schema: &Value, instance: &mut Value) {
    if let (Some(members), Value::String(text)) =
        (schema.get("enum").and_then(Value::as_array), &*instance)
    {
        let exact = members.iter().any(|m| m.as_str() == Some(text.as_str()));
        if !exact {
            let replacement = members
                .iter()
                .filter_map(Value::as_str)
                .find(|member| member.eq_ignore_ascii_case(text))
                .map(str::to_owned);
            if let Some(member) = replacement {
                *instance = Value::String(member);
            }
        }
        return;
    }
    match instance {
        Value::Object(map) => {
            let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
                return;
            };
            for (key, child) in map.iter_mut() {
                if let Some(child_schema) = properties.get(key) {
                    normalise_enum_case(child_schema, child);
                }
            }
        }
        Value::Array(items) => {
            if let Some(item_schema) = schema.get("items").filter(|s| s.is_object()) {
                for item in items {
                    normalise_enum_case(item_schema, item);
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_contract_and_the_probe_schema_compile() {
        for contract in Contract::ALL {
            assert!(
                contract_schema(contract).is_compiled(),
                "{} did not compile",
                contract.name()
            );
        }
        assert!(probe_schema().is_compiled());
    }

    #[test]
    fn extracts_the_first_complete_value() {
        let cases = [
            (r#"{"a":1}"#, json!({"a": 1})),
            ("```json\n{\"a\": 1}\n```", json!({"a": 1})),
            ("```\n{\"a\": 1}\n```\nHope this helps!", json!({"a": 1})),
            (
                "Sure! Here you go: {\"a\": {\"b\": [1, 2]}} and {\"a\": 2}",
                json!({"a": {"b": [1, 2]}}),
            ),
            ("note [see below] {\"a\": 1}", json!({"a": 1})),
            ("[1, 2, 3]", json!([1, 2, 3])),
            (r#"{"a":"```"} trailing"#, json!({"a": "```"})),
        ];
        for (text, expected) in cases {
            assert_eq!(extract_json(text), Some(expected), "{text}");
        }
    }

    #[test]
    fn no_json_means_none() {
        for text in [
            "",
            "I cannot do that.",
            "{\"a\": 1",
            "42",
            "\"just a string\"",
        ] {
            assert_eq!(extract_json(text), None, "{text}");
        }
    }

    #[test]
    fn enum_case_is_normalised_only_where_the_schema_has_an_enum() {
        let schema = &contract_schema(Contract::TurnAnalysis).full;
        let mut value = json!({
            "turns": [{
                "turn_seq": 1,
                "errors": [{
                    "category": "Verb_Tense",
                    "quote": "He go",
                    "correction": "He goes",
                    "severity": "MAJOR",
                    "addressed_in_reply": false
                }],
                "objective_evidence": [{"objective_id": "OBJ-Greeting", "status": "Demonstrated", "quote": "hi"}],
                "understood_tutor": "Yes",
                "note_for_next_turn": "Keep Going"
            }]
        });
        normalise_enum_case(schema, &mut value);
        let turn = &value["turns"][0];
        assert_eq!(turn["errors"][0]["category"], "verb_tense");
        assert_eq!(turn["errors"][0]["severity"], "major");
        assert_eq!(turn["errors"][0]["quote"], "He go");
        assert_eq!(turn["objective_evidence"][0]["status"], "demonstrated");
        assert_eq!(
            turn["objective_evidence"][0]["objective_id"],
            "OBJ-Greeting"
        );
        assert_eq!(turn["understood_tutor"], "yes");
        assert_eq!(turn["note_for_next_turn"], "Keep Going");
        assert!(
            contract_schema(Contract::TurnAnalysis)
                .check_value(&value)
                .is_ok()
        );
    }

    #[test]
    fn a_value_that_is_not_an_enum_member_stays_invalid() {
        let schema = contract_schema(Contract::RubricScore);
        let mut value = json!({
            "dimension_scores": [{"dimension": "vocabulary", "band": "5", "evidence_quotes": [], "reason": "x"}],
            "content_points": [], "on_task": true, "feedback_en": "ok", "feedback_l1": "ok"
        });
        normalise_enum_case(&schema.full, &mut value);
        let violations = schema.check_value(&value).expect_err("two bad enums");
        let paths: Vec<&str> = violations.iter().map(|v| v.path.as_str()).collect();
        assert!(
            paths.contains(&"/dimension_scores/0/dimension"),
            "{paths:?}"
        );
        assert!(paths.contains(&"/dimension_scores/0/band"), "{paths:?}");
    }

    #[test]
    fn violations_carry_json_pointers() {
        let schema = probe_schema();
        let violations = schema
            .check_value(&json!({"title": 5, "word_count": 3}))
            .expect_err("invalid");
        let paths: Vec<&str> = violations.iter().map(|v| v.path.as_str()).collect();
        assert!(paths.contains(&"/title"), "{paths:?}");
        assert!(
            violations.iter().any(|v| v.path.is_empty()),
            "the missing property is reported at the root: {violations:?}"
        );
    }

    #[test]
    fn extra_properties_are_rejected_because_the_schemas_close_every_object() {
        let schema = probe_schema();
        assert!(
            schema
                .check_value(&json!({"title": "t", "word_count": 1, "is_ok": true, "extra": 1}))
                .is_err()
        );
        assert!(
            schema
                .check_value(&json!({"title": "t", "word_count": 1, "is_ok": true}))
                .is_ok()
        );
        assert!(
            schema
                .check_value(&json!({"title": "t", "word_count": 1.5, "is_ok": true}))
                .is_err()
        );
    }

    #[test]
    fn check_text_reports_why() {
        let schema = probe_schema();
        assert_eq!(
            schema
                .check_text("no json here")
                .expect_err("invalid")
                .reason,
            InvalidReason::NotJson
        );
        assert_eq!(
            schema
                .check_text("{\"title\": 1}")
                .expect_err("invalid")
                .reason,
            InvalidReason::SchemaMismatch
        );
        let ok = schema
            .check_text("```json\n{\"title\":\"t\",\"word_count\":1,\"is_ok\":false}\n```")
            .expect("valid");
        assert_eq!(ok["is_ok"], false);
    }
}
