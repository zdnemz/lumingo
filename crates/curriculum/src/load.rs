use crate::Unit;
use jsonschema::Validator;
use serde_json::Value;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

const UNIT_SCHEMA: &str = include_str!("../../../curriculum/schema/unit.schema.json");
/// How many schema problems one report lists.
const MAX_ISSUES: usize = 20;

/// One schema violation. `path` is a JSON pointer into the unit, for example `/activities/3/options`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaIssue {
    pub path: String,
    pub message: String,
}

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("not valid JSON at line {line}, column {column}")]
    Json { line: usize, column: usize },
    #[error("the unit does not match the schema: {}", summarize(.0))]
    Schema(Vec<SchemaIssue>),
    /// The schema accepted it but the types did not: the schema and the model have drifted apart.
    #[error("the unit passed the schema but does not fit the types: {0}")]
    Model(String),
    #[error("could not read {0}")]
    Io(PathBuf),
}

fn summarize(issues: &[SchemaIssue]) -> String {
    issues
        .iter()
        .take(3)
        .map(|i| {
            format!(
                "{} ({})",
                if i.path.is_empty() { "/" } else { &i.path },
                i.message
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Builds a validator for each `act_*` definition, keyed by the `type` values it allows.
fn activity_validators(schema: &Value) -> HashMap<String, Validator> {
    let mut map = HashMap::new();
    let Some(defs) = schema.get("$defs").and_then(Value::as_object) else {
        return map;
    };
    for (name, def) in defs.iter().filter(|(n, _)| n.starts_with("act_")) {
        let type_spec = def.pointer("/properties/type");
        let types: Vec<String> = match (
            type_spec.and_then(|t| t.get("const")),
            type_spec.and_then(|t| t.get("enum")),
        ) {
            (Some(c), _) => c.as_str().map(str::to_owned).into_iter().collect(),
            (_, Some(Value::Array(e))) => e
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect(),
            _ => Vec::new(),
        };
        let wrapper = serde_json::json!({ "$ref": format!("#/$defs/{name}"), "$defs": defs });
        if let Ok(validator) = jsonschema::validator_for(&wrapper) {
            for t in types {
                map.insert(t, validator.clone());
            }
        }
    }
    map
}

/// Holds the compiled unit schema. Compiling takes a moment, so build one and reuse it.
pub struct UnitLoader {
    validator: Validator,
    /// One validator per activity `type`. The schema picks an activity's shape with
    /// `oneOf`, which reports only "no variant matched"; these say what is wrong.
    activity: HashMap<String, Validator>,
}

impl Default for UnitLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl UnitLoader {
    pub fn new() -> Self {
        // The schema is part of this crate and is checked by the test suite, so a
        // failure here is a build defect, not a runtime condition.
        #[allow(clippy::expect_used)]
        let schema: Value =
            serde_json::from_str(UNIT_SCHEMA).expect("bundled unit schema is valid JSON");
        #[allow(clippy::expect_used)]
        let validator = jsonschema::validator_for(&schema).expect("bundled unit schema compiles");
        Self {
            validator,
            activity: activity_validators(&schema),
        }
    }

    /// Schema problems of an already parsed unit. Empty when it is valid. A problem
    /// inside an activity is reported at the exact field, for example `/activities/3/answer_index`.
    pub fn issues(&self, unit: &Value) -> Vec<SchemaIssue> {
        let mut out = Vec::new();
        for e in self.validator.iter_errors(unit) {
            let path = e.instance_path().to_string();
            match self.activity_issues(unit, &path) {
                Some(detail) => out.extend(detail),
                None => out.push(SchemaIssue {
                    path,
                    message: e.to_string(),
                }),
            }
            if out.len() >= MAX_ISSUES {
                out.truncate(MAX_ISSUES);
                break;
            }
        }
        out
    }

    /// For an error at `/activities/N`, validates that activity against the schema of
    /// its own `type`. `None` when the path is not an activity or the type is unknown.
    fn activity_issues(&self, unit: &Value, path: &str) -> Option<Vec<SchemaIssue>> {
        let index: usize = path.strip_prefix("/activities/")?.parse().ok()?;
        let activity = unit.get("activities")?.get(index)?;
        let validator = self.activity.get(activity.get("type")?.as_str()?)?;
        let detail: Vec<SchemaIssue> = validator
            .iter_errors(activity)
            .map(|e| SchemaIssue {
                path: format!("{path}{}", e.instance_path()),
                message: e.to_string(),
            })
            .collect();
        (!detail.is_empty()).then_some(detail)
    }

    pub fn load_value(&self, value: Value) -> Result<Unit, LoadError> {
        let issues = self.issues(&value);
        if !issues.is_empty() {
            return Err(LoadError::Schema(issues));
        }
        serde_json::from_value(value).map_err(|e| LoadError::Model(e.to_string()))
    }

    pub fn load_str(&self, text: &str) -> Result<Unit, LoadError> {
        let value: Value = serde_json::from_str(text).map_err(|e| LoadError::Json {
            line: e.line(),
            column: e.column(),
        })?;
        self.load_value(value)
    }

    pub fn load_file(&self, path: &Path) -> Result<Unit, LoadError> {
        let text = fs::read_to_string(path).map_err(|_| LoadError::Io(path.to_path_buf()))?;
        self.load_str(&text)
    }
}

/// The result for one file.
pub type LoadedFile = (PathBuf, Result<Unit, LoadError>);

/// Loads every `*.json` file under `dir` (recursively), sorted by path. Each file
/// gets its own result so one broken unit does not hide the others.
pub fn load_dir(loader: &UnitLoader, dir: &Path) -> Result<Vec<LoadedFile>, LoadError> {
    fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), LoadError> {
        for entry in fs::read_dir(dir)
            .map_err(|_| LoadError::Io(dir.to_path_buf()))?
            .flatten()
        {
            let path = entry.path();
            if path.is_dir() {
                collect(&path, out)?;
            } else if path.extension().is_some_and(|x| x == "json") {
                out.push(path);
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    collect(dir, &mut files)?;
    files.sort();
    Ok(files
        .into_iter()
        .map(|p| {
            let r = loader.load_file(&p);
            (p, r)
        })
        .collect())
}
