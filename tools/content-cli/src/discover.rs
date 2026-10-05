//! Reading files and folders, deciding what each file is, and running the rules.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use curriculum::syllabus::Syllabus;
use curriculum::validate::catalogs::{SKIPPED_DIRECTORIES, detect_kind, validate_catalog};
use curriculum::validate::{
    FileKind, FileReport, Finding, RuleCode, SetConfig, UnitInput, UnitOptions, WordLevels,
    validate_set,
};
use curriculum::{Level, sha256_hex};
use serde_json::Value;

use crate::{Outcome, ValidateOptions};

/// One JSON file as read from disk.
struct SourceFile {
    label: String,
    kind: FileKind,
    checksum: Option<String>,
    document: Result<Value, String>,
}

/// Runs the validators on everything the options name.
pub fn validate(options: &ValidateOptions) -> Result<Outcome> {
    let (paths, any_folder) = gather(&options.paths)?;
    let sources: Vec<SourceFile> = paths
        .iter()
        .map(|path| read_source(path, options.kind))
        .collect();

    let word_levels = match &options.word_list {
        Some(path) => {
            let text = fs::read_to_string(path)
                .with_context(|| format!("cannot read the word list {}", path.display()))?;
            Some(
                WordLevels::parse(&text)
                    .with_context(|| format!("the word list {} is not valid", path.display()))?,
            )
        }
        None => None,
    };

    let root = options
        .curriculum_root
        .clone()
        .or_else(|| find_curriculum_root(&options.paths));
    let mut setup_findings: Vec<Finding> = Vec::new();
    let syllabi = match &root {
        Some(root) => load_syllabi(root, &mut setup_findings),
        None => HashMap::new(),
    };
    let rubric_ids = root
        .as_ref()
        .and_then(|root| load_rubric_ids(root, &mut setup_findings));

    let mut unit_inputs: Vec<UnitInput> = Vec::new();
    let mut unit_checksums: Vec<Option<String>> = Vec::new();
    let mut catalog_sources: Vec<&SourceFile> = Vec::new();
    for source in &sources {
        if source.kind == FileKind::Unit {
            unit_inputs.push(UnitInput {
                file: source.label.clone(),
                document: source.document.clone(),
            });
            unit_checksums.push(source.checksum.clone());
        } else {
            catalog_sources.push(source);
        }
    }

    let config = SetConfig {
        unit_options: UnitOptions {
            word_levels: word_levels.as_ref(),
            grammar_check: None,
        },
        whole_set: any_folder,
        complete: options.complete,
        syllabi: &syllabi,
        rubric_ids: rubric_ids.as_ref(),
    };
    let mut report = validate_set(&unit_inputs, &config);
    for (unit_report, checksum) in report.units.iter_mut().zip(unit_checksums) {
        unit_report.checksum = checksum;
    }
    for source in catalog_sources {
        let mut file_report = match &source.document {
            Ok(document) => {
                validate_catalog(source.kind, &source.label, document, options.complete)
            }
            Err(reason) => FileReport {
                file: source.label.clone(),
                kind: source.kind,
                findings: vec![Finding::new(RuleCode::K01, "", reason.clone())],
                ..FileReport::default()
            },
        };
        file_report.checksum = source.checksum.clone();
        report.catalogs.push(file_report);
    }
    for finding in setup_findings {
        report.set.push(finding);
    }

    Ok(Outcome {
        files_examined: sources.len(),
        report,
        curriculum_root: root,
    })
}

/// The files to validate and whether any argument was a folder.
fn gather(arguments: &[PathBuf]) -> Result<(Vec<PathBuf>, bool)> {
    let mut files: BTreeSet<PathBuf> = BTreeSet::new();
    let mut any_folder = false;
    for argument in arguments {
        let metadata = fs::metadata(argument)
            .with_context(|| format!("cannot open {}", argument.display()))?;
        if metadata.is_dir() {
            any_folder = true;
            walk(argument, &mut files)?;
        } else {
            files.insert(argument.clone());
        }
    }
    Ok((files.into_iter().collect(), any_folder))
}

fn walk(dir: &Path, out: &mut BTreeSet<PathBuf>) -> Result<()> {
    let entries =
        fs::read_dir(dir).with_context(|| format!("cannot read folder {}", dir.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| format!("cannot read folder {}", dir.display()))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let file_type = entry
            .file_type()
            .with_context(|| format!("cannot inspect {}", path.display()))?;
        if file_type.is_dir() {
            if !SKIPPED_DIRECTORIES.contains(&name.as_str()) && !name.starts_with('.') {
                walk(&path, out)?;
            }
        } else if path.extension().is_some_and(|ext| ext == "json") {
            out.insert(path);
        }
    }
    Ok(())
}

fn label(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

fn read_source(path: &Path, forced: Option<FileKind>) -> SourceFile {
    let kind = forced.unwrap_or_else(|| detect_kind(path));
    let label = label(path);
    match fs::read(path) {
        Err(err) => SourceFile {
            label,
            kind,
            checksum: None,
            document: Err(format!("cannot read the file: {err}")),
        },
        Ok(bytes) => {
            let checksum = Some(sha256_hex(&bytes));
            let document = curriculum::load::parse_json(&bytes).map_err(|err| err.to_string());
            SourceFile {
                label,
                kind,
                checksum,
                document,
            }
        }
    }
}

/// The folder that holds `syllabus/` and `catalogs/`: the first ancestor of the first path
/// that has either of them.
fn find_curriculum_root(paths: &[PathBuf]) -> Option<PathBuf> {
    let first = paths.first()?;
    let absolute = fs::canonicalize(first).ok()?;
    absolute
        .ancestors()
        .find(|dir| dir.join("catalogs").is_dir() || dir.join("syllabus").is_dir())
        .map(Path::to_path_buf)
}

fn json_files_in(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    files
}

/// Syllabi by level from `<root>/syllabus/*.json`. A file that cannot be used becomes a
/// finding, because rules X03 and X05 would otherwise be weaker without anyone noticing.
fn load_syllabi(root: &Path, problems: &mut Vec<Finding>) -> HashMap<Level, Syllabus> {
    let mut syllabi = HashMap::new();
    for path in json_files_in(&root.join("syllabus")) {
        let outcome = fs::read(&path)
            .map_err(|err| err.to_string())
            .and_then(|bytes| {
                curriculum::syllabus::load_syllabus_bytes(&bytes).map_err(|err| err.to_string())
            });
        match outcome {
            Ok(syllabus) => {
                syllabi.insert(syllabus.level, syllabus);
            }
            Err(reason) => problems.push(Finding::new(
                RuleCode::K01,
                "",
                format!(
                    "the syllabus {} cannot be used for X03: {reason}",
                    label(&path)
                ),
            )),
        }
    }
    syllabi
}

/// Rubric ids from `<root>/catalogs/rubrics/*.json`, or `None` when that folder does not exist.
fn load_rubric_ids(root: &Path, problems: &mut Vec<Finding>) -> Option<HashSet<String>> {
    let dir = root.join("catalogs").join("rubrics");
    if !dir.is_dir() {
        return None;
    }
    let mut ids = HashSet::new();
    for path in json_files_in(&dir) {
        let id = fs::read(&path)
            .map_err(|err| err.to_string())
            .and_then(|bytes| curriculum::load::parse_json(&bytes).map_err(|e| e.to_string()))
            .and_then(|value| {
                value
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .ok_or_else(|| "it has no id".to_owned())
            });
        match id {
            Ok(id) => {
                ids.insert(id);
            }
            Err(reason) => problems.push(Finding::new(
                RuleCode::K01,
                "",
                format!(
                    "the rubric {} cannot be used for X06: {reason}",
                    label(&path)
                ),
            )),
        }
    }
    Some(ids)
}

/// Parses a `--kind` value.
pub fn parse_kind(name: &str) -> Result<FileKind> {
    match FileKind::parse(name) {
        Some(kind) => Ok(kind),
        None => bail!(
            "unknown kind \"{name}\"; use one of: {}",
            FileKind::ALL
                .iter()
                .map(|kind| kind.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}
