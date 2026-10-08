//! Key-value application settings (`app_settings`). Never holds a key or any
//! learner text.

use serde::{Deserialize, Serialize};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use crate::db::Database;
use crate::error::Result;
use crate::row::timestamp_col;
use crate::time::Timestamp;

/// One stored setting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Setting {
    pub key: String,
    pub value: String,
    pub updated_at: Timestamp,
}

impl Setting {
    fn from_row(row: &SqliteRow) -> Result<Self> {
        Ok(Self {
            key: row.try_get("key")?,
            value: row.try_get("value")?,
            updated_at: timestamp_col(row, "updated_at")?,
        })
    }
}

/// Queries on the `app_settings` table.
pub struct Settings<'a> {
    db: &'a Database,
}

impl Database {
    /// Settings queries.
    pub fn settings(&self) -> Settings<'_> {
        Settings { db: self }
    }
}

impl Settings<'_> {
    /// The value stored under `key`.
    pub async fn get(&self, key: &str) -> Result<Option<String>> {
        let value = sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?1")
            .bind(key)
            .fetch_optional(self.db.reader())
            .await?;
        Ok(value)
    }

    /// Stores `value` under `key`, replacing any earlier value.
    pub async fn set(&self, key: &str, value: &str, updated_at: &Timestamp) -> Result<()> {
        sqlx::query(
            "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, ?3) \
             ON CONFLICT (key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        )
        .bind(key)
        .bind(value)
        .bind(updated_at.to_string())
        .execute(self.db.writer())
        .await?;
        Ok(())
    }

    /// Removes a setting. Returns whether it existed.
    pub async fn remove(&self, key: &str) -> Result<bool> {
        let result = sqlx::query("DELETE FROM app_settings WHERE key = ?1")
            .bind(key)
            .execute(self.db.writer())
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Every setting, ordered by key.
    pub async fn all(&self) -> Result<Vec<Setting>> {
        let rows = sqlx::query("SELECT * FROM app_settings ORDER BY key")
            .fetch_all(self.db.reader())
            .await?;
        rows.iter().map(Setting::from_row).collect()
    }
}
