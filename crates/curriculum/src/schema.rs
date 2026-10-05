//! JSON Schema checking with error paths a content author can act on.

use std::sync::OnceLock;

use jsonschema::error::{ValidationError, ValidationErrorKind};
use serde_json::Value;

/// The unit schema, embedded so the loader works without a path to the repository.
pub const UNIT_SCHEMA_JSON: &str = include_str!("../../../curriculum/schema/unit.schema.json");

/// Longest message kept per issue. The validator prints whole instances in some messages.
const MAX_MESSAGE_CHARS: usize = 240;

/// One schema violation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaIssue {
    /// JSON pointer of the offending value inside the document, `""` for the root.
    pub pointer: String,
    /// The schema keyword that failed, for example `required` or `pattern`.
    pub keyword: String,
    pub message: String,
}

/// A schema that could not be turned into a checker. This is a defect of the schema file.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("schema {name} cannot be used: {reason}")]
pub struct SchemaLoadError {
    pub name: String,
    pub reason: String,
}

/// A compiled JSON Schema that reports every violation with its JSON pointer.
pub struct SchemaChecker {
    validator: jsonschema::Validator,
}

impl std::fmt::Debug for SchemaChecker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SchemaChecker").finish_non_exhaustive()
    }
}

impl SchemaChecker {
    /// Compiles `schema`. `name` only labels the error.
    pub fn new(name: &str, schema: &Value) -> Result<Self, SchemaLoadError> {
        let validator = jsonschema::validator_for(schema).map_err(|err| SchemaLoadError {
            name: name.to_owned(),
            reason: err.to_string(),
        })?;
        Ok(Self { validator })
    }

    /// Compiles a schema given as JSON text.
    pub fn from_json(name: &str, schema_json: &str) -> Result<Self, SchemaLoadError> {
        let schema: Value = serde_json::from_str(schema_json).map_err(|err| SchemaLoadError {
            name: name.to_owned(),
            reason: err.to_string(),
        })?;
        Self::new(name, &schema)
    }

    /// Every violation of `instance`, in the order the validator found them.
    pub fn issues(&self, instance: &Value) -> Vec<SchemaIssue> {
        let mut out = Vec::new();
        for error in self.validator.iter_errors(instance) {
            flatten(&error, &mut out);
        }
        out
    }

    pub fn is_valid(&self, instance: &Value) -> bool {
        self.validator.is_valid(instance)
    }
}

/// The compiled unit schema, built once per process.
pub fn unit_checker() -> Result<&'static SchemaChecker, SchemaLoadError> {
    static CHECKER: OnceLock<Result<SchemaChecker, SchemaLoadError>> = OnceLock::new();
    CHECKER
        .get_or_init(|| SchemaChecker::from_json("unit.schema.json", UNIT_SCHEMA_JSON))
        .as_ref()
        .map_err(Clone::clone)
}

/// Turns one validator error into issues. A `oneOf` over typed alternatives, such as the
/// activity list, would otherwise print one error per alternative. When the alternatives
/// are told apart by a `type` field, only the errors of the alternative the document
/// chose are kept.
fn flatten(error: &ValidationError<'_>, out: &mut Vec<SchemaIssue>) {
    let pointer = error.instance_path().to_string();
    match error.kind() {
        ValidationErrorKind::OneOfNotValid { context } => {
            let type_pointer = format!("{pointer}/type");
            let chosen: Vec<&Vec<ValidationError<'static>>> = context
                .iter()
                .filter(|branch| {
                    !branch
                        .iter()
                        .any(|e| e.instance_path().to_string() == type_pointer)
                })
                .collect();
            let discriminated = chosen.len() < context.len();
            if discriminated {
                if let Some(best) = chosen.into_iter().min_by_key(|branch| branch.len()) {
                    for inner in best {
                        flatten(inner, out);
                    }
                } else {
                    out.push(SchemaIssue {
                        pointer: type_pointer,
                        keyword: "oneOf".to_owned(),
                        message: "type is not one of the known types".to_owned(),
                    });
                }
            } else {
                let alternatives: Vec<String> = context
                    .iter()
                    .map(|branch| {
                        branch
                            .iter()
                            .map(|e| truncate(&e.to_string(), 120))
                            .collect::<Vec<_>>()
                            .join(" and ")
                    })
                    .collect();
                out.push(SchemaIssue {
                    pointer,
                    keyword: "oneOf".to_owned(),
                    message: format!(
                        "none of the allowed alternatives is satisfied: {}",
                        alternatives.join("; or ")
                    ),
                });
            }
        }
        ValidationErrorKind::OneOfMultipleValid { .. } => out.push(SchemaIssue {
            pointer,
            keyword: "oneOf".to_owned(),
            message: "more than one of the allowed alternatives is present; exactly one is allowed"
                .to_owned(),
        }),
        kind => out.push(SchemaIssue {
            pointer,
            keyword: kind.keyword().to_owned(),
            message: truncate(&error.to_string(), MAX_MESSAGE_CHARS),
        }),
    }
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let mut short: String = text.chars().take(max_chars).collect();
    short.push_str("...");
    short
}
