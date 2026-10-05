//! LLM provider profiles. The database records which provider is configured and
//! where its key is read from (`key_source`). The key itself is never stored
//! here; there is no column for it.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use crate::db::Database;
use crate::enums::{KeySource, ProviderProtocol};
use crate::error::Result;
use crate::row::{
    enum_col, expect_changed, json_text, opt_json_col, opt_timestamp_col, timestamp_col,
};
use crate::time::Timestamp;

/// A stored provider profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderProfile {
    pub id: i64,
    pub name: String,
    pub protocol: ProviderProtocol,
    pub base_url: String,
    pub model: String,
    pub key_source: KeySource,
    /// What the last probe found the provider can do.
    pub capabilities: Option<Value>,
    pub probed_at: Option<Timestamp>,
    /// Set when the scorer qualification test passed.
    pub qualified_at: Option<Timestamp>,
    pub is_active: bool,
    pub created_at: Timestamp,
}

/// A provider profile to create. It starts inactive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewProviderProfile {
    pub name: String,
    pub protocol: ProviderProtocol,
    pub base_url: String,
    pub model: String,
    pub key_source: KeySource,
    pub created_at: Timestamp,
}

impl ProviderProfile {
    fn from_row(row: &SqliteRow) -> Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            protocol: enum_col(row, "protocol")?,
            base_url: row.try_get("base_url")?,
            model: row.try_get("model")?,
            key_source: enum_col(row, "key_source")?,
            capabilities: opt_json_col(row, "capabilities_json")?,
            probed_at: opt_timestamp_col(row, "probed_at")?,
            qualified_at: opt_timestamp_col(row, "qualified_at")?,
            is_active: row.try_get("is_active")?,
            created_at: timestamp_col(row, "created_at")?,
        })
    }
}

/// Queries on the `provider_profiles` table.
pub struct Providers<'a> {
    db: &'a Database,
}

impl Database {
    /// Provider profile queries.
    pub fn providers(&self) -> Providers<'_> {
        Providers { db: self }
    }
}

impl Providers<'_> {
    /// Creates an inactive profile. Names are unique.
    pub async fn create(&self, new: &NewProviderProfile) -> Result<ProviderProfile> {
        let row = sqlx::query(
            "INSERT INTO provider_profiles (name, protocol, base_url, model, key_source, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) RETURNING *",
        )
        .bind(&new.name)
        .bind(new.protocol.as_str())
        .bind(&new.base_url)
        .bind(&new.model)
        .bind(new.key_source.as_str())
        .bind(new.created_at.to_string())
        .fetch_one(self.db.writer())
        .await?;
        ProviderProfile::from_row(&row)
    }

    /// One profile.
    pub async fn get(&self, id: i64) -> Result<Option<ProviderProfile>> {
        let row = sqlx::query("SELECT * FROM provider_profiles WHERE id = ?1")
            .bind(id)
            .fetch_optional(self.db.reader())
            .await?;
        row.as_ref().map(ProviderProfile::from_row).transpose()
    }

    /// Every profile, ordered by name.
    pub async fn list(&self) -> Result<Vec<ProviderProfile>> {
        let rows = sqlx::query("SELECT * FROM provider_profiles ORDER BY name")
            .fetch_all(self.db.reader())
            .await?;
        rows.iter().map(ProviderProfile::from_row).collect()
    }

    /// The active profile, if one is set.
    pub async fn active(&self) -> Result<Option<ProviderProfile>> {
        let row = sqlx::query("SELECT * FROM provider_profiles WHERE is_active = 1")
            .fetch_optional(self.db.reader())
            .await?;
        row.as_ref().map(ProviderProfile::from_row).transpose()
    }

    /// Makes one profile the active one. At most one is active: a partial unique
    /// index enforces it and this switch happens in one transaction.
    pub async fn set_active(&self, id: i64) -> Result<()> {
        let mut tx = self.db.begin_write().await?;
        sqlx::query("UPDATE provider_profiles SET is_active = 0 WHERE is_active = 1")
            .execute(&mut *tx)
            .await?;
        let result = sqlx::query("UPDATE provider_profiles SET is_active = 1 WHERE id = ?1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        // An unknown id returns here and the transaction rolls back, so the
        // previously active profile stays active.
        expect_changed(&result, "provider profile")?;
        tx.commit().await?;
        Ok(())
    }

    /// Stores what a probe found out.
    pub async fn record_probe(
        &self,
        id: i64,
        capabilities: &Value,
        probed_at: &Timestamp,
    ) -> Result<()> {
        let result = sqlx::query(
            "UPDATE provider_profiles SET capabilities_json = ?2, probed_at = ?3 WHERE id = ?1",
        )
        .bind(id)
        .bind(json_text(capabilities))
        .bind(probed_at.to_string())
        .execute(self.db.writer())
        .await?;
        expect_changed(&result, "provider profile")
    }

    /// Records that the scorer qualification test passed (or clears it).
    pub async fn set_qualified(&self, id: i64, qualified_at: Option<&Timestamp>) -> Result<()> {
        let result = sqlx::query("UPDATE provider_profiles SET qualified_at = ?2 WHERE id = ?1")
            .bind(id)
            .bind(qualified_at.map(ToString::to_string))
            .execute(self.db.writer())
            .await?;
        expect_changed(&result, "provider profile")
    }

    /// Removes a profile. Past `llm_calls` keep their numbers with a null
    /// provider; sessions keep their summaries with a null provider.
    pub async fn delete(&self, id: i64) -> Result<()> {
        let result = sqlx::query("DELETE FROM provider_profiles WHERE id = ?1")
            .bind(id)
            .execute(self.db.writer())
            .await?;
        expect_changed(&result, "provider profile")
    }
}
