//! The learner profile. Version 1 creates exactly one row; every learner table
//! carries `profile_id` so more profiles need UI work but no data migration.

use serde::{Deserialize, Serialize};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use crate::db::Database;
use crate::enums::{L1HelpMode, UiLanguage};
use crate::error::Result;
use crate::row::{enum_col, expect_changed, timestamp_col};
use crate::time::Timestamp;

/// A stored profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub id: i64,
    pub display_name: String,
    pub ui_language: UiLanguage,
    /// First language of the learner, an ISO 639-1 code such as `id`.
    pub l1: String,
    pub l1_help_mode: L1HelpMode,
    pub created_at: Timestamp,
}

/// A profile to create.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewProfile {
    pub display_name: String,
    pub ui_language: UiLanguage,
    pub l1: String,
    pub l1_help_mode: L1HelpMode,
    pub created_at: Timestamp,
}

impl Profile {
    fn from_row(row: &SqliteRow) -> Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            display_name: row.try_get("display_name")?,
            ui_language: enum_col(row, "ui_language")?,
            l1: row.try_get("l1")?,
            l1_help_mode: enum_col(row, "l1_help_mode")?,
            created_at: timestamp_col(row, "created_at")?,
        })
    }
}

/// Queries on the `profiles` table.
pub struct Profiles<'a> {
    db: &'a Database,
}

impl Database {
    /// Profile queries.
    pub fn profiles(&self) -> Profiles<'_> {
        Profiles { db: self }
    }
}

impl Profiles<'_> {
    /// Creates a profile and returns it with its id.
    pub async fn create(&self, new: &NewProfile) -> Result<Profile> {
        let row = sqlx::query(
            "INSERT INTO profiles (display_name, ui_language, l1, l1_help_mode, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5) RETURNING *",
        )
        .bind(&new.display_name)
        .bind(new.ui_language.as_str())
        .bind(&new.l1)
        .bind(new.l1_help_mode.as_str())
        .bind(new.created_at.to_string())
        .fetch_one(self.db.writer())
        .await?;
        Profile::from_row(&row)
    }

    /// The profile with this id, if it exists.
    pub async fn get(&self, id: i64) -> Result<Option<Profile>> {
        let row = sqlx::query("SELECT * FROM profiles WHERE id = ?1")
            .bind(id)
            .fetch_optional(self.db.reader())
            .await?;
        row.as_ref().map(Profile::from_row).transpose()
    }

    /// The oldest profile. Version 1 has exactly one.
    pub async fn first(&self) -> Result<Option<Profile>> {
        let row = sqlx::query("SELECT * FROM profiles ORDER BY id LIMIT 1")
            .fetch_optional(self.db.reader())
            .await?;
        row.as_ref().map(Profile::from_row).transpose()
    }

    /// Replaces the editable fields of a profile.
    pub async fn update(&self, profile: &Profile) -> Result<()> {
        let result = sqlx::query(
            "UPDATE profiles SET display_name = ?2, ui_language = ?3, l1 = ?4, l1_help_mode = ?5 \
             WHERE id = ?1",
        )
        .bind(profile.id)
        .bind(&profile.display_name)
        .bind(profile.ui_language.as_str())
        .bind(&profile.l1)
        .bind(profile.l1_help_mode.as_str())
        .execute(self.db.writer())
        .await?;
        expect_changed(&result, "profile")
    }

    /// Deletes the profile and, by cascade, everything that belongs to the
    /// learner: sessions, turns, attempts, evidence, estimates, progress and the
    /// game layer. This is "delete all learning data".
    ///
    /// The caller removes the audio files first (see
    /// `AudioClips::paths_for_profile`) and calls [`Database::compact`] afterwards
    /// so the deleted text does not stay in unused pages of the file.
    pub async fn delete(&self, id: i64) -> Result<()> {
        let result = sqlx::query("DELETE FROM profiles WHERE id = ?1")
            .bind(id)
            .execute(self.db.writer())
            .await?;
        expect_changed(&result, "profile")
    }
}
