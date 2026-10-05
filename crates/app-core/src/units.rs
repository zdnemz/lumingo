//! The curriculum index.
//!
//! The units are JSON files in a folder that ships with the program. At start-up
//! the core loads every file through `curriculum` (schema check first), keeps a
//! small summary of each in memory, and writes an index of units and objectives
//! to the database, because progress and attempts point at unit ids. The full
//! unit is read from its file when asked for, and its checksum must still match
//! the indexed one, so the UI never shows a file that differs from what the
//! scores were recorded against.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{PoisonError, RwLock};

use curriculum::{LoadedUnit, SkillCounts, Unit, UnitFile, load_unit_bytes, sha256_hex};
use storage::{
    Database, IndexStatus, Level, NewCurriculumVersion, NewObjective, NewUnit, Timestamp,
};
use tokio::sync::Mutex;

use crate::api::{UnitDetail, UnitIssue, UnitList, UnitSummary};
use crate::core::AppCore;
use crate::error::{CoreError, CoreResult};

/// Longest message kept for one skipped file. Schema failures can list many
/// problems; the first lines are enough to find the file's fault.
const MAX_ISSUE_CHARS: usize = 400;

pub(crate) struct UnitHub {
    index: RwLock<Index>,
    /// One re-index at a time.
    reindex: Mutex<()>,
}

#[derive(Default)]
struct Index {
    content_version: Option<String>,
    summaries: Vec<UnitSummary>,
    issues: Vec<UnitIssue>,
    files: HashMap<String, IndexedFile>,
}

struct IndexedFile {
    path: PathBuf,
    checksum: String,
}

/// What a scan of the folder found.
struct Scan {
    units: Vec<(PathBuf, LoadedUnit)>,
    issues: Vec<UnitIssue>,
}

fn storage_level(level: curriculum::Level) -> Level {
    match level {
        curriculum::Level::A1 => Level::A1,
        curriculum::Level::A2 => Level::A2,
        curriculum::Level::B1 => Level::B1,
        curriculum::Level::B2 => Level::B2,
        curriculum::Level::C1 => Level::C1,
        curriculum::Level::C2 => Level::C2,
    }
}

fn shorten(text: &str) -> String {
    if text.chars().count() <= MAX_ISSUE_CHARS {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(MAX_ISSUE_CHARS).collect();
    cut.push('…');
    cut
}

fn relative(dir: &Path, path: &Path) -> String {
    path.strip_prefix(dir)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Loads every unit file under `dir`. Runs on a blocking thread.
fn scan(dir: &Path) -> Scan {
    let files = match curriculum::load_unit_dir(dir) {
        Ok(files) => files,
        Err(error) => {
            return Scan {
                units: Vec::new(),
                issues: vec![UnitIssue {
                    file: ".".to_owned(),
                    // `LoadError::Io` names the folder, which is the learner's own path.
                    message: shorten(&format!("the units folder cannot be read: {error}")),
                }],
            };
        }
    };
    let mut units: Vec<(PathBuf, LoadedUnit)> = Vec::new();
    let mut issues = Vec::new();
    let mut ids = HashSet::new();
    let mut positions = HashSet::new();
    for UnitFile { path, result } in files {
        let file = relative(dir, &path);
        match result {
            Err(error) => issues.push(UnitIssue {
                file,
                message: shorten(&error.to_string()),
            }),
            Ok(loaded) => {
                let unit = &loaded.unit;
                if !ids.insert(unit.id.clone()) {
                    issues.push(UnitIssue {
                        file,
                        message: format!("the unit id {} is already used by another file", unit.id),
                    });
                } else if !positions.insert((unit.level, unit.sequence)) {
                    issues.push(UnitIssue {
                        file,
                        message: format!(
                            "level {} position {} is already taken by another unit",
                            unit.level, unit.sequence
                        ),
                    });
                } else {
                    units.push((path, loaded));
                }
            }
        }
    }
    units.sort_by(|a, b| {
        let (a, b) = (&a.1.unit, &b.1.unit);
        (a.level, a.sequence, &a.id).cmp(&(b.level, b.sequence, &b.id))
    });
    Scan { units, issues }
}

fn summary(unit: &Unit) -> UnitSummary {
    UnitSummary {
        id: unit.id.clone(),
        level: unit.level,
        sequence: unit.sequence,
        title: unit.title.clone(),
        theme: unit.theme.clone(),
        estimated_minutes: unit.estimated_minutes,
        skill_counts: SkillCounts::of_unit(unit),
        objective_count: u32::try_from(unit.objectives.len()).unwrap_or(u32::MAX),
        prerequisites: unit.prerequisites.clone(),
    }
}

/// SHA-256 over `<id> <file checksum>` lines in id order: it changes when a
/// unit is added, removed or edited.
fn manifest_checksum(units: &[(PathBuf, LoadedUnit)]) -> String {
    let mut lines: Vec<String> = units
        .iter()
        .map(|(_, loaded)| format!("{} {}\n", loaded.unit.id, loaded.checksum))
        .collect();
    lines.sort();
    sha256_hex(lines.concat().as_bytes())
}

/// Writes the index when this set of files was not indexed before. Returns the
/// content version that names the set.
async fn write_index(
    db: &Database,
    units: &[(PathBuf, LoadedUnit)],
    now: &Timestamp,
) -> CoreResult<Option<String>> {
    if units.is_empty() {
        return Ok(None);
    }
    let manifest = manifest_checksum(units);
    // The version name is derived from the manifest, so an edited file is a new
    // version and the table's "same version, different checksum" case cannot
    // arise from this code.
    let content_version = format!("manifest-{}", &manifest[..12]);
    match db
        .curriculum()
        .index_status(&content_version, &manifest)
        .await?
    {
        IndexStatus::Current => {}
        IndexStatus::Changed => {
            tracing::warn!("the unit index has this version with another checksum; not rewritten");
        }
        IndexStatus::Missing => {
            let new_units = units
                .iter()
                .map(|(_, loaded)| {
                    let unit = &loaded.unit;
                    NewUnit {
                        id: unit.id.clone(),
                        level: storage_level(unit.level),
                        sequence: i64::from(unit.sequence),
                        title_en: unit.title.en.clone(),
                        file_sha256: loaded.checksum.clone(),
                        objectives: unit
                            .objectives
                            .iter()
                            .map(|objective| NewObjective {
                                id: format!("{}/{}", unit.id, objective.id),
                                skill: serde_json::to_value(objective.skill)
                                    .ok()
                                    .and_then(|v| v.as_str().map(str::to_owned))
                                    .unwrap_or_default(),
                                can_do_en: objective.can_do.en.clone(),
                            })
                            .collect(),
                    }
                })
                .collect();
            let schema_version = units
                .first()
                .map(|(_, loaded)| loaded.unit.schema_version.clone())
                .unwrap_or_default();
            db.curriculum()
                .install(&NewCurriculumVersion {
                    content_version: content_version.clone(),
                    schema_version,
                    manifest_sha256: manifest,
                    installed_at: *now,
                    units: new_units,
                })
                .await?;
            tracing::info!(units = units.len(), "indexed the curriculum");
        }
    }
    Ok(Some(content_version))
}

impl UnitHub {
    pub(crate) async fn load(dir: &Path, db: &Database, now: &Timestamp) -> CoreResult<Self> {
        let hub = Self {
            index: RwLock::new(Index::default()),
            reindex: Mutex::new(()),
        };
        hub.rebuild(dir, db, now).await?;
        Ok(hub)
    }

    async fn rebuild(&self, dir: &Path, db: &Database, now: &Timestamp) -> CoreResult<()> {
        let owned = dir.to_owned();
        let Scan { units, mut issues } = tokio::task::spawn_blocking(move || scan(&owned))
            .await
            .map_err(|_| CoreError::Internal("reading the unit files did not finish".to_owned()))?;
        let content_version = match write_index(db, &units, now).await {
            Ok(version) => version,
            Err(error) => {
                // The files are still good to show; progress cannot be recorded
                // against them until the index is written, which the message says.
                tracing::warn!(error = %error, "the unit index could not be written");
                issues.push(UnitIssue {
                    file: ".".to_owned(),
                    message: "the unit index could not be written to the database".to_owned(),
                });
                None
            }
        };
        let summaries = units
            .iter()
            .map(|(_, loaded)| summary(&loaded.unit))
            .collect();
        let files = units
            .into_iter()
            .map(|(path, loaded)| {
                (
                    loaded.unit.id,
                    IndexedFile {
                        path,
                        checksum: loaded.checksum,
                    },
                )
            })
            .collect();
        *self.index.write().unwrap_or_else(PoisonError::into_inner) = Index {
            content_version,
            summaries,
            issues,
            files,
        };
        Ok(())
    }

    fn list(&self) -> UnitList {
        let index = self.index.read().unwrap_or_else(PoisonError::into_inner);
        UnitList {
            content_version: index.content_version.clone(),
            units: index.summaries.clone(),
            issues: index.issues.clone(),
        }
    }

    fn file_of(&self, id: &str) -> Option<(PathBuf, String)> {
        let index = self.index.read().unwrap_or_else(PoisonError::into_inner);
        index
            .files
            .get(id)
            .map(|file| (file.path.clone(), file.checksum.clone()))
    }
}

impl AppCore {
    /// The units in learning order, with the files that were skipped.
    pub fn list_units(&self) -> UnitList {
        self.units.list()
    }

    /// One whole unit, read from its file. The file must still be the one that
    /// was indexed.
    pub async fn unit(&self, id: &str) -> CoreResult<UnitDetail> {
        let (path, checksum) = self
            .units
            .file_of(id)
            .ok_or(CoreError::NotFound { what: "unit" })?;
        let bytes = tokio::fs::read(&path)
            .await
            .map_err(|error| CoreError::io("reading the unit file", &error))?;
        if sha256_hex(&bytes) != checksum {
            return Err(CoreError::Conflict(
                "the unit file changed after it was indexed; restart the program".to_owned(),
            ));
        }
        let loaded = tokio::task::spawn_blocking(move || load_unit_bytes(&bytes))
            .await
            .map_err(|_| CoreError::Internal("reading the unit did not finish".to_owned()))?
            .map_err(|error| CoreError::Internal(error.to_string()))?;
        Ok(UnitDetail { unit: loaded.unit })
    }

    /// Reads the unit folder again and updates the index. Start-up does this
    /// once; call it after changing the files while the program runs.
    pub async fn reindex_curriculum(&self) -> CoreResult<UnitList> {
        self.ensure_running()?;
        let _one_at_a_time = self.units.reindex.lock().await;
        self.units
            .rebuild(&self.config.curriculum_dir, &self.db, &self.clock().now())
            .await?;
        Ok(self.units.list())
    }
}
