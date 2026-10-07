//! The one learner profile. v1 creates exactly one row; every learner table
//! carries `profile_id` so multi-profile support needs no data migration.

use crate::db::Database;
use crate::error::{StorageError, classify, not_found};
use crate::models::{NewProfile, Profile};
use crate::rows::ProfileRow;

impl Database {
    /// Creates the profile and returns the stored row.
    pub async fn create_profile(&self, profile: NewProfile) -> Result<Profile, StorageError> {
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO profiles (display_name, ui_language, l1, l1_help_mode, created_at) \
             VALUES (?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(&profile.display_name)
        .bind(profile.ui_language.as_str())
        .bind(&profile.l1)
        .bind(profile.l1_help_mode.as_str())
        .bind(&profile.created_at)
        .fetch_one(self.writer())
        .await
        .map_err(|error| classify("profiles", error))?;

        self.profile(id).await
    }

    /// Reads one profile. `NotFound` when the id does not exist.
    pub async fn profile(&self, id: i64) -> Result<Profile, StorageError> {
        let row: Option<ProfileRow> = sqlx::query_as(
            "SELECT id, display_name, ui_language, l1, l1_help_mode, created_at \
             FROM profiles WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.readers())
        .await?;

        row.map(Profile::try_from)
            .transpose()?
            .ok_or_else(|| not_found("profiles", id))
    }

    /// Every profile, oldest first. v1 has one row; the API does not assume it.
    pub async fn profiles(&self) -> Result<Vec<Profile>, StorageError> {
        let rows: Vec<ProfileRow> = sqlx::query_as(
            "SELECT id, display_name, ui_language, l1, l1_help_mode, created_at \
             FROM profiles ORDER BY id",
        )
        .fetch_all(self.readers())
        .await?;

        rows.into_iter().map(Profile::try_from).collect()
    }

    /// Deletes the profile and, by cascade, everything that belongs to the
    /// learner. Recordings listed in `audio_clips` must be deleted from disk by
    /// the caller first; see [`audio_clips_for_profile`].
    ///
    /// [`audio_clips_for_profile`]: Database::audio_clips_for_profile
    pub async fn delete_learner_data(&self, profile_id: i64) -> Result<(), StorageError> {
        let result = sqlx::query("DELETE FROM profiles WHERE id = ?")
            .bind(profile_id)
            .execute(self.writer())
            .await
            .map_err(|error| classify("profiles", error))?;

        if result.rows_affected() == 0 {
            return Err(not_found("profiles", profile_id));
        }
        Ok(())
    }
}
