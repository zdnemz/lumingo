//! The curriculum index: the database rows that point at unit files.
//!
//! The units themselves are JSON files that ship with the program; the database
//! keeps an index of them and their checksums, so a session or an attempt can
//! point at a unit that is not open, and the app can tell whether the files on
//! disk are the ones it indexed.
//!
//! This repository takes plain data. It does not depend on the `curriculum`
//! crate: the caller maps a loaded unit to [`NewIndexedUnit`]. That keeps the
//! two crates independent and lets the index be tested without unit files.

use crate::db::Database;
use crate::error::{StorageError, classify};
use crate::models::Level;

/// One installed set of unit files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurriculumVersion {
    pub id: i64,
    pub content_version: String,
    pub schema_version: String,
    pub unit_count: i64,
    /// SHA-256 of the manifest that lists every unit file.
    pub manifest_sha256: String,
    pub installed_at: String,
}

/// An objective to index. Its id is `<unit id>/<objective id>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewIndexedObjective {
    pub id: String,
    pub skill: String,
    pub can_do_en: String,
}

/// A unit to index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewIndexedUnit {
    pub id: String,
    pub level: Level,
    /// Position inside the level, 1 to 30.
    pub sequence: i64,
    pub title_en: String,
    /// SHA-256 of the unit file.
    pub file_sha256: String,
    pub objectives: Vec<NewIndexedObjective>,
}

/// A set of units to index in one transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewCurriculumVersion {
    pub content_version: String,
    pub schema_version: String,
    pub manifest_sha256: String,
    pub installed_at: String,
    pub units: Vec<NewIndexedUnit>,
}

/// A unit as indexed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedUnit {
    pub id: String,
    pub curriculum_version_id: i64,
    pub level: Level,
    pub sequence: i64,
    pub title_en: String,
    pub file_sha256: String,
}

/// An objective as indexed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedObjective {
    pub id: String,
    pub unit_id: String,
    pub skill: String,
    pub can_do_en: String,
}

/// How the files on disk compare with the index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexStatus {
    /// This content version was never indexed.
    Missing,
    /// Indexed, and the manifest checksum matches.
    Current,
    /// Same content version, different manifest checksum: the files changed
    /// without a version bump.
    Changed,
}

impl Database {
    /// Indexes a curriculum version and its units in one transaction.
    ///
    /// A unit that is already indexed is updated to point at the new version and
    /// its objectives are replaced. A failure rolls everything back, so the
    /// previous index survives intact.
    pub async fn install_curriculum(
        &self,
        new: &NewCurriculumVersion,
    ) -> Result<CurriculumVersion, StorageError> {
        for unit in &new.units {
            let prefix = format!("{}/", unit.id);
            if unit.objectives.iter().any(|o| !o.id.starts_with(&prefix)) {
                return Err(StorageError::Invalid {
                    table: "objectives",
                    detail: "an objective id must be its unit id, a slash, and the local id"
                        .to_owned(),
                });
            }
        }
        let unit_count = i64::try_from(new.units.len()).map_err(|_| StorageError::Invalid {
            table: "curriculum_versions",
            detail: "too many units in one version".to_owned(),
        })?;

        let mut tx = self.writer().begin().await?;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO curriculum_versions \
             (content_version, schema_version, unit_count, manifest_sha256, installed_at) \
             VALUES (?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(&new.content_version)
        .bind(&new.schema_version)
        .bind(unit_count)
        .bind(&new.manifest_sha256)
        .bind(&new.installed_at)
        .fetch_one(&mut *tx)
        .await
        .map_err(|error| classify("curriculum_versions", error))?;

        for unit in &new.units {
            sqlx::query(
                "INSERT INTO units (id, curriculum_version_id, level, sequence, title_en, file_sha256) \
                 VALUES (?, ?, ?, ?, ?, ?) \
                 ON CONFLICT (id) DO UPDATE SET \
                   curriculum_version_id = excluded.curriculum_version_id, \
                   level = excluded.level, sequence = excluded.sequence, \
                   title_en = excluded.title_en, file_sha256 = excluded.file_sha256",
            )
            .bind(&unit.id)
            .bind(id)
            .bind(unit.level.as_str())
            .bind(unit.sequence)
            .bind(&unit.title_en)
            .bind(&unit.file_sha256)
            .execute(&mut *tx)
            .await
            .map_err(|error| classify("units", error))?;

            sqlx::query("DELETE FROM objectives WHERE unit_id = ?")
                .bind(&unit.id)
                .execute(&mut *tx)
                .await?;
            for objective in &unit.objectives {
                sqlx::query(
                    "INSERT INTO objectives (id, unit_id, skill, can_do_en) VALUES (?, ?, ?, ?)",
                )
                .bind(&objective.id)
                .bind(&unit.id)
                .bind(&objective.skill)
                .bind(&objective.can_do_en)
                .execute(&mut *tx)
                .await
                .map_err(|error| classify("objectives", error))?;
            }
        }
        tx.commit().await?;

        self.curriculum_version(&new.content_version)
            .await?
            .ok_or_else(|| StorageError::Invalid {
                table: "curriculum_versions",
                detail: "the row vanished after the insert".to_owned(),
            })
    }

    /// One installed version by its content version name.
    pub async fn curriculum_version(
        &self,
        content_version: &str,
    ) -> Result<Option<CurriculumVersion>, StorageError> {
        let row: Option<VersionRow> = sqlx::query_as(
            "SELECT id, content_version, schema_version, unit_count, manifest_sha256, installed_at \
             FROM curriculum_versions WHERE content_version = ?",
        )
        .bind(content_version)
        .fetch_optional(self.readers())
        .await?;
        Ok(row.map(Into::into))
    }

    /// The most recently installed version.
    pub async fn latest_curriculum_version(
        &self,
    ) -> Result<Option<CurriculumVersion>, StorageError> {
        let row: Option<VersionRow> = sqlx::query_as(
            "SELECT id, content_version, schema_version, unit_count, manifest_sha256, installed_at \
             FROM curriculum_versions ORDER BY id DESC LIMIT 1",
        )
        .fetch_optional(self.readers())
        .await?;
        Ok(row.map(Into::into))
    }

    /// Compares a content version and its manifest checksum with the index.
    pub async fn curriculum_index_status(
        &self,
        content_version: &str,
        manifest_sha256: &str,
    ) -> Result<IndexStatus, StorageError> {
        let stored: Option<String> = sqlx::query_scalar(
            "SELECT manifest_sha256 FROM curriculum_versions WHERE content_version = ?",
        )
        .bind(content_version)
        .fetch_optional(self.readers())
        .await?;
        Ok(match stored {
            None => IndexStatus::Missing,
            Some(sha) if sha == manifest_sha256 => IndexStatus::Current,
            Some(_) => IndexStatus::Changed,
        })
    }

    /// One indexed unit.
    pub async fn indexed_unit(&self, id: &str) -> Result<Option<IndexedUnit>, StorageError> {
        let row: Option<UnitRow> = sqlx::query_as(
            "SELECT id, curriculum_version_id, level, sequence, title_en, file_sha256 \
             FROM units WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.readers())
        .await?;
        row.map(IndexedUnit::try_from).transpose()
    }

    /// Every indexed unit, in curriculum order.
    pub async fn indexed_units(&self) -> Result<Vec<IndexedUnit>, StorageError> {
        let rows: Vec<UnitRow> = sqlx::query_as(
            "SELECT id, curriculum_version_id, level, sequence, title_en, file_sha256 \
             FROM units ORDER BY level, sequence",
        )
        .fetch_all(self.readers())
        .await?;
        rows.into_iter().map(IndexedUnit::try_from).collect()
    }

    /// The objectives of one unit, ordered by id.
    pub async fn unit_objectives(
        &self,
        unit_id: &str,
    ) -> Result<Vec<IndexedObjective>, StorageError> {
        let rows: Vec<ObjectiveRow> = sqlx::query_as(
            "SELECT id, unit_id, skill, can_do_en FROM objectives WHERE unit_id = ? ORDER BY id",
        )
        .bind(unit_id)
        .fetch_all(self.readers())
        .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// `(unit id, file checksum)` for every indexed unit, so the loader can
    /// find the files that changed on disk.
    pub async fn unit_checksums(&self) -> Result<Vec<(String, String)>, StorageError> {
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT id, file_sha256 FROM units ORDER BY id")
                .fetch_all(self.readers())
                .await?;
        Ok(rows)
    }
}

#[derive(sqlx::FromRow)]
struct VersionRow {
    id: i64,
    content_version: String,
    schema_version: String,
    unit_count: i64,
    manifest_sha256: String,
    installed_at: String,
}

impl From<VersionRow> for CurriculumVersion {
    fn from(row: VersionRow) -> Self {
        Self {
            id: row.id,
            content_version: row.content_version,
            schema_version: row.schema_version,
            unit_count: row.unit_count,
            manifest_sha256: row.manifest_sha256,
            installed_at: row.installed_at,
        }
    }
}

#[derive(sqlx::FromRow)]
struct UnitRow {
    id: String,
    curriculum_version_id: i64,
    level: String,
    sequence: i64,
    title_en: String,
    file_sha256: String,
}

impl TryFrom<UnitRow> for IndexedUnit {
    type Error = StorageError;

    fn try_from(row: UnitRow) -> Result<Self, Self::Error> {
        let level = row
            .level
            .parse()
            .map_err(|error: crate::models::UnknownValue| StorageError::Invalid {
                table: "units",
                detail: error.to_string(),
            })?;
        Ok(IndexedUnit {
            id: row.id,
            curriculum_version_id: row.curriculum_version_id,
            level,
            sequence: row.sequence,
            title_en: row.title_en,
            file_sha256: row.file_sha256,
        })
    }
}

#[derive(sqlx::FromRow)]
struct ObjectiveRow {
    id: String,
    unit_id: String,
    skill: String,
    can_do_en: String,
}

impl From<ObjectiveRow> for IndexedObjective {
    fn from(row: ObjectiveRow) -> Self {
        Self {
            id: row.id,
            unit_id: row.unit_id,
            skill: row.skill,
            can_do_en: row.can_do_en,
        }
    }
}
