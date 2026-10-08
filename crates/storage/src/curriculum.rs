//! The curriculum index. The units and catalogs themselves are JSON files that
//! ship with the program; the database keeps an index of them and their
//! checksums, so the program can tell whether the files on disk are the ones it
//! indexed.

use serde::{Deserialize, Serialize};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use crate::db::Database;
use crate::enums::Level;
use crate::error::{Result, StorageError};
use crate::row::{enum_col, timestamp_col};
use crate::time::Timestamp;

/// One installed version of the curriculum.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurriculumVersion {
    pub id: i64,
    pub content_version: String,
    pub schema_version: String,
    pub unit_count: i64,
    /// SHA-256 of the manifest that lists every unit file.
    pub manifest_sha256: String,
    pub installed_at: Timestamp,
}

/// An objective to index. Its id is `<unit id>/<objective id>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewObjective {
    pub id: String,
    pub skill: String,
    pub can_do_en: String,
}

/// A unit to index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewUnit {
    pub id: String,
    pub level: Level,
    /// Position inside the level, 1 to 30.
    pub sequence: i64,
    pub title_en: String,
    /// SHA-256 of the unit file.
    pub file_sha256: String,
    pub objectives: Vec<NewObjective>,
}

/// A curriculum version to index, with its units.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewCurriculumVersion {
    pub content_version: String,
    pub schema_version: String,
    pub manifest_sha256: String,
    pub installed_at: Timestamp,
    pub units: Vec<NewUnit>,
}

/// A unit as indexed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexedUnit {
    pub id: String,
    pub curriculum_version_id: i64,
    pub level: Level,
    pub sequence: i64,
    pub title_en: String,
    pub file_sha256: String,
}

/// An objective as indexed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexedObjective {
    pub id: String,
    pub unit_id: String,
    pub skill: String,
    pub can_do_en: String,
}

/// How the files on disk compare with the index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IndexStatus {
    /// This content version was never indexed.
    Missing,
    /// Indexed, and the manifest checksum matches.
    Current,
    /// Same content version, different manifest checksum: the files changed
    /// without a version bump.
    Changed,
}

impl CurriculumVersion {
    fn from_row(row: &SqliteRow) -> Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            content_version: row.try_get("content_version")?,
            schema_version: row.try_get("schema_version")?,
            unit_count: row.try_get("unit_count")?,
            manifest_sha256: row.try_get("manifest_sha256")?,
            installed_at: timestamp_col(row, "installed_at")?,
        })
    }
}

impl IndexedUnit {
    fn from_row(row: &SqliteRow) -> Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            curriculum_version_id: row.try_get("curriculum_version_id")?,
            level: enum_col(row, "level")?,
            sequence: row.try_get("sequence")?,
            title_en: row.try_get("title_en")?,
            file_sha256: row.try_get("file_sha256")?,
        })
    }
}

impl IndexedObjective {
    fn from_row(row: &SqliteRow) -> Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            unit_id: row.try_get("unit_id")?,
            skill: row.try_get("skill")?,
            can_do_en: row.try_get("can_do_en")?,
        })
    }
}

/// Queries on the curriculum index tables.
pub struct Curriculum<'a> {
    db: &'a Database,
}

impl Database {
    /// Curriculum index queries.
    pub fn curriculum(&self) -> Curriculum<'_> {
        Curriculum { db: self }
    }
}

impl Curriculum<'_> {
    /// Indexes a curriculum version and its units in one transaction.
    ///
    /// A unit that is already indexed is updated to point at the new version and
    /// its objectives are replaced. Units missing from the new version are left
    /// alone, because sessions and progress may still refer to them. A failure
    /// leaves the previous index untouched.
    pub async fn install(&self, new: &NewCurriculumVersion) -> Result<CurriculumVersion> {
        for unit in &new.units {
            let prefix = format!("{}/", unit.id);
            if unit.objectives.iter().any(|o| !o.id.starts_with(&prefix)) {
                return Err(StorageError::Rule(
                    "an objective id starts with its unit id and a slash",
                ));
            }
        }
        let unit_count = i64::try_from(new.units.len())
            .map_err(|_| StorageError::Rule("too many units in one curriculum version"))?;

        let mut tx = self.db.begin_write().await?;
        let row = sqlx::query(
            "INSERT INTO curriculum_versions \
             (content_version, schema_version, unit_count, manifest_sha256, installed_at) \
             VALUES (?1, ?2, ?3, ?4, ?5) RETURNING *",
        )
        .bind(&new.content_version)
        .bind(&new.schema_version)
        .bind(unit_count)
        .bind(&new.manifest_sha256)
        .bind(new.installed_at.to_string())
        .fetch_one(&mut *tx)
        .await?;
        let version = CurriculumVersion::from_row(&row)?;

        for unit in &new.units {
            sqlx::query(
                "INSERT INTO units (id, curriculum_version_id, level, sequence, title_en, file_sha256) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
                 ON CONFLICT (id) DO UPDATE SET curriculum_version_id = excluded.curriculum_version_id, \
                 level = excluded.level, sequence = excluded.sequence, \
                 title_en = excluded.title_en, file_sha256 = excluded.file_sha256",
            )
            .bind(&unit.id)
            .bind(version.id)
            .bind(unit.level.as_str())
            .bind(unit.sequence)
            .bind(&unit.title_en)
            .bind(&unit.file_sha256)
            .execute(&mut *tx)
            .await?;
            sqlx::query("DELETE FROM objectives WHERE unit_id = ?1")
                .bind(&unit.id)
                .execute(&mut *tx)
                .await?;
            for objective in &unit.objectives {
                sqlx::query(
                    "INSERT INTO objectives (id, unit_id, skill, can_do_en) VALUES (?1, ?2, ?3, ?4)",
                )
                .bind(&objective.id)
                .bind(&unit.id)
                .bind(&objective.skill)
                .bind(&objective.can_do_en)
                .execute(&mut *tx)
                .await?;
            }
        }
        tx.commit().await?;
        Ok(version)
    }

    /// Compares a content version and its manifest checksum with the index.
    pub async fn index_status(
        &self,
        content_version: &str,
        manifest_sha256: &str,
    ) -> Result<IndexStatus> {
        let stored: Option<String> = sqlx::query_scalar(
            "SELECT manifest_sha256 FROM curriculum_versions WHERE content_version = ?1",
        )
        .bind(content_version)
        .fetch_optional(self.db.reader())
        .await?;
        Ok(match stored {
            None => IndexStatus::Missing,
            Some(sha) if sha == manifest_sha256 => IndexStatus::Current,
            Some(_) => IndexStatus::Changed,
        })
    }

    /// The most recently installed version.
    pub async fn latest_version(&self) -> Result<Option<CurriculumVersion>> {
        let row = sqlx::query("SELECT * FROM curriculum_versions ORDER BY id DESC LIMIT 1")
            .fetch_optional(self.db.reader())
            .await?;
        row.as_ref().map(CurriculumVersion::from_row).transpose()
    }

    /// One indexed unit.
    pub async fn unit(&self, id: &str) -> Result<Option<IndexedUnit>> {
        let row = sqlx::query("SELECT * FROM units WHERE id = ?1")
            .bind(id)
            .fetch_optional(self.db.reader())
            .await?;
        row.as_ref().map(IndexedUnit::from_row).transpose()
    }

    /// Every indexed unit in curriculum order.
    pub async fn units(&self) -> Result<Vec<IndexedUnit>> {
        let rows = sqlx::query("SELECT * FROM units ORDER BY level, sequence")
            .fetch_all(self.db.reader())
            .await?;
        rows.iter().map(IndexedUnit::from_row).collect()
    }

    /// The objectives of a unit, ordered by id.
    pub async fn objectives(&self, unit_id: &str) -> Result<Vec<IndexedObjective>> {
        let rows = sqlx::query("SELECT * FROM objectives WHERE unit_id = ?1 ORDER BY id")
            .bind(unit_id)
            .fetch_all(self.db.reader())
            .await?;
        rows.iter().map(IndexedObjective::from_row).collect()
    }

    /// `(unit id, file checksum)` for every indexed unit, so the loader can find
    /// the files that changed on disk.
    pub async fn unit_checksums(&self) -> Result<Vec<(String, String)>> {
        let rows = sqlx::query("SELECT id, file_sha256 FROM units ORDER BY id")
            .fetch_all(self.db.reader())
            .await?;
        rows.iter()
            .map(|row| Ok((row.try_get("id")?, row.try_get("file_sha256")?)))
            .collect()
    }
}
