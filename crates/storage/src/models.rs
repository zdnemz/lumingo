//! Speech models installed on this machine (`models_installed`). The weights
//! are files in the model directory; this table records which ones, with their
//! checksums and licenses, for the model manager.

use serde::{Deserialize, Serialize};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use crate::db::Database;
use crate::enums::ModelRole;
use crate::error::Result;
use crate::row::{enum_col, timestamp_col};
use crate::time::Timestamp;

/// An installed model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledModel {
    /// Id from `models/manifest.toml`.
    pub id: String,
    pub role: ModelRole,
    pub version: String,
    pub path: String,
    pub sha256: String,
    pub size_bytes: i64,
    pub license: String,
    pub installed_at: Timestamp,
}

impl InstalledModel {
    fn from_row(row: &SqliteRow) -> Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            role: enum_col(row, "role")?,
            version: row.try_get("version")?,
            path: row.try_get("path")?,
            sha256: row.try_get("sha256")?,
            size_bytes: row.try_get("size_bytes")?,
            license: row.try_get("license")?,
            installed_at: timestamp_col(row, "installed_at")?,
        })
    }
}

/// Queries on the `models_installed` table.
pub struct Models<'a> {
    db: &'a Database,
}

impl Database {
    /// Installed-model queries.
    pub fn models(&self) -> Models<'_> {
        Models { db: self }
    }
}

impl Models<'_> {
    /// Records a model, replacing the row of the same id (a re-download).
    pub async fn upsert(&self, model: &InstalledModel) -> Result<()> {
        sqlx::query(
            "INSERT INTO models_installed \
             (id, role, version, path, sha256, size_bytes, license, installed_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
             ON CONFLICT (id) DO UPDATE SET role = excluded.role, version = excluded.version, \
             path = excluded.path, sha256 = excluded.sha256, size_bytes = excluded.size_bytes, \
             license = excluded.license, installed_at = excluded.installed_at",
        )
        .bind(&model.id)
        .bind(model.role.as_str())
        .bind(&model.version)
        .bind(&model.path)
        .bind(&model.sha256)
        .bind(model.size_bytes)
        .bind(&model.license)
        .bind(model.installed_at.to_string())
        .execute(self.db.writer())
        .await?;
        Ok(())
    }

    /// One installed model.
    pub async fn get(&self, id: &str) -> Result<Option<InstalledModel>> {
        let row = sqlx::query("SELECT * FROM models_installed WHERE id = ?1")
            .bind(id)
            .fetch_optional(self.db.reader())
            .await?;
        row.as_ref().map(InstalledModel::from_row).transpose()
    }

    /// Every installed model, ordered by role and id.
    pub async fn list(&self) -> Result<Vec<InstalledModel>> {
        let rows = sqlx::query("SELECT * FROM models_installed ORDER BY role, id")
            .fetch_all(self.db.reader())
            .await?;
        rows.iter().map(InstalledModel::from_row).collect()
    }

    /// The installed models of one role.
    pub async fn by_role(&self, role: ModelRole) -> Result<Vec<InstalledModel>> {
        let rows = sqlx::query("SELECT * FROM models_installed WHERE role = ?1 ORDER BY id")
            .bind(role.as_str())
            .fetch_all(self.db.reader())
            .await?;
        rows.iter().map(InstalledModel::from_row).collect()
    }

    /// Forgets a model. Returns whether it was recorded.
    pub async fn remove(&self, id: &str) -> Result<bool> {
        let result = sqlx::query("DELETE FROM models_installed WHERE id = ?1")
            .bind(id)
            .execute(self.db.writer())
            .await?;
        Ok(result.rows_affected() > 0)
    }
}
