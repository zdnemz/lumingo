//! Loading unit files: schema check first, typed model second, checksum alongside.

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::index::UnitIndexEntry;
use crate::model::Unit;
use crate::schema::{SchemaIssue, SchemaLoadError, unit_checker};

/// Why a unit file could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("cannot read {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("not valid JSON at line {line}, column {column}: {message}")]
    Json {
        line: usize,
        column: usize,
        message: String,
    },
    #[error("{}", format_issues(issues))]
    Schema { issues: Vec<SchemaIssue> },
    /// The document passed the schema but does not fit the Rust model. This means the
    /// schema file and `model.rs` have drifted apart, which is a defect of the crate.
    #[error("the unit passed the schema but does not fit the Rust model: {0}")]
    Model(String),
    #[error(transparent)]
    SchemaUnavailable(#[from] SchemaLoadError),
}

impl LoadError {
    /// The schema issues, when the failure was a schema failure.
    pub fn schema_issues(&self) -> &[SchemaIssue] {
        match self {
            LoadError::Schema { issues } => issues,
            _ => &[],
        }
    }
}

fn format_issues(issues: &[SchemaIssue]) -> String {
    let mut text = format!("{} schema error(s)", issues.len());
    for issue in issues {
        let pointer = if issue.pointer.is_empty() {
            "(document)"
        } else {
            issue.pointer.as_str()
        };
        // Writing to a String cannot fail.
        let _ = write!(text, "\n  {pointer}: {}", issue.message);
    }
    text
}

/// A unit that passed the schema, with the checksum of the bytes it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedUnit {
    pub unit: Unit,
    /// SHA-256 of the file bytes, lower-case hex.
    pub checksum: String,
}

impl LoadedUnit {
    /// The row `app-core` stores for this unit.
    pub fn index_entry(&self) -> UnitIndexEntry {
        UnitIndexEntry::new(&self.unit, &self.checksum)
    }
}

/// SHA-256 of `bytes` as 64 lower-case hex characters.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        // Writing to a String cannot fail.
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// Parses `bytes` as JSON, with the line and column of a syntax error.
pub fn parse_json(bytes: &[u8]) -> Result<Value, LoadError> {
    serde_json::from_slice(bytes).map_err(|err| LoadError::Json {
        line: err.line(),
        column: err.column(),
        message: err.to_string(),
    })
}

/// Checks `value` against the unit schema and, when it passes, converts it to a [`Unit`].
pub fn unit_from_value(value: &Value) -> Result<Unit, LoadError> {
    let issues = unit_checker()?.issues(value);
    if !issues.is_empty() {
        return Err(LoadError::Schema { issues });
    }
    serde_json::from_value(value.clone()).map_err(|err| LoadError::Model(err.to_string()))
}

/// Loads a unit from the bytes of a file.
pub fn load_unit_bytes(bytes: &[u8]) -> Result<LoadedUnit, LoadError> {
    let value = parse_json(bytes)?;
    let unit = unit_from_value(&value)?;
    Ok(LoadedUnit {
        unit,
        checksum: sha256_hex(bytes),
    })
}

/// Loads one unit file.
pub fn load_unit_file(path: &Path) -> Result<LoadedUnit, LoadError> {
    let bytes = read_file(path)?;
    load_unit_bytes(&bytes)
}

/// The outcome for one file of a directory load.
#[derive(Debug)]
pub struct UnitFile {
    pub path: PathBuf,
    pub result: Result<LoadedUnit, LoadError>,
}

/// Loads every `.json` file under `dir`, in path order. One bad file does not stop the
/// others: each file carries its own result.
pub fn load_unit_dir(dir: &Path) -> Result<Vec<UnitFile>, LoadError> {
    let mut paths = Vec::new();
    collect_json_files(dir, &mut paths)?;
    paths.sort();
    Ok(paths
        .into_iter()
        .map(|path| {
            let result = load_unit_file(&path);
            UnitFile { path, result }
        })
        .collect())
}

/// Every `.json` file under `dir`, recursively, unsorted.
pub fn collect_json_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), LoadError> {
    let entries = fs::read_dir(dir).map_err(|source| LoadError::Io {
        path: dir.to_owned(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| LoadError::Io {
            path: dir.to_owned(),
            source,
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|source| LoadError::Io {
            path: path.clone(),
            source,
        })?;
        if file_type.is_dir() {
            collect_json_files(&path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "json") {
            out.push(path);
        }
    }
    Ok(())
}

/// Reads a whole file, labelling a failure with its path.
pub fn read_file(path: &Path) -> Result<Vec<u8>, LoadError> {
    fs::read(path).map_err(|source| LoadError::Io {
        path: path.to_owned(),
        source,
    })
}
