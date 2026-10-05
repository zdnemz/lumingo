//! Sessions: one lesson, conversation, chat, writing task, reading task, drill,
//! review, checkpoint or placement run.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use crate::db::Database;
use crate::enums::{SessionKind, SessionMode, SessionStatus};
use crate::error::{Result, StorageError};
use crate::row::{
    enum_col, expect_changed, opt_enum_col, opt_json_col, opt_json_text, opt_timestamp_col,
    timestamp_col,
};
use crate::time::Timestamp;

/// A stored session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub id: i64,
    pub profile_id: i64,
    pub kind: SessionKind,
    pub unit_id: Option<String>,
    pub activity_id: Option<String>,
    pub mode: Option<SessionMode>,
    pub status: SessionStatus,
    pub provider_profile_id: Option<i64>,
    /// Counts and flags written when the session ends. Never learner text.
    pub summary: Option<Value>,
    pub app_version: String,
    pub started_at: Timestamp,
    pub ended_at: Option<Timestamp>,
}

/// A session to start. It is created `active`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewSession {
    pub profile_id: i64,
    pub kind: SessionKind,
    pub unit_id: Option<String>,
    pub activity_id: Option<String>,
    pub mode: Option<SessionMode>,
    pub provider_profile_id: Option<i64>,
    pub app_version: String,
    pub started_at: Timestamp,
}

impl Session {
    fn from_row(row: &SqliteRow) -> Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            profile_id: row.try_get("profile_id")?,
            kind: enum_col(row, "kind")?,
            unit_id: row.try_get("unit_id")?,
            activity_id: row.try_get("activity_id")?,
            mode: opt_enum_col(row, "mode")?,
            status: enum_col(row, "status")?,
            provider_profile_id: row.try_get("provider_profile_id")?,
            summary: opt_json_col(row, "summary_json")?,
            app_version: row.try_get("app_version")?,
            started_at: timestamp_col(row, "started_at")?,
            ended_at: opt_timestamp_col(row, "ended_at")?,
        })
    }
}

/// Queries on the `sessions` table.
pub struct Sessions<'a> {
    db: &'a Database,
}

impl Database {
    /// Session queries.
    pub fn sessions(&self) -> Sessions<'_> {
        Sessions { db: self }
    }
}

impl Sessions<'_> {
    /// Starts a session.
    pub async fn create(&self, new: &NewSession) -> Result<Session> {
        let row = sqlx::query(
            "INSERT INTO sessions \
             (profile_id, kind, unit_id, activity_id, mode, provider_profile_id, app_version, started_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) RETURNING *",
        )
        .bind(new.profile_id)
        .bind(new.kind.as_str())
        .bind(&new.unit_id)
        .bind(&new.activity_id)
        .bind(new.mode.map(|m| m.as_str()))
        .bind(new.provider_profile_id)
        .bind(&new.app_version)
        .bind(new.started_at.to_string())
        .fetch_one(self.db.writer())
        .await?;
        Session::from_row(&row)
    }

    /// The session with this id, if it exists.
    pub async fn get(&self, id: i64) -> Result<Option<Session>> {
        let row = sqlx::query("SELECT * FROM sessions WHERE id = ?1")
            .bind(id)
            .fetch_optional(self.db.reader())
            .await?;
        row.as_ref().map(Session::from_row).transpose()
    }

    /// A profile's sessions, newest first.
    pub async fn list_for_profile(&self, profile_id: i64, limit: u32) -> Result<Vec<Session>> {
        let rows = sqlx::query(
            "SELECT * FROM sessions WHERE profile_id = ?1 \
             ORDER BY started_at DESC, id DESC LIMIT ?2",
        )
        .bind(profile_id)
        .bind(i64::from(limit))
        .fetch_all(self.db.reader())
        .await?;
        rows.iter().map(Session::from_row).collect()
    }

    /// Ends an active session as `Completed` or `Aborted`.
    ///
    /// A session ends once: ending it again is refused so a late cancel cannot
    /// overwrite the outcome of a session that already completed.
    pub async fn finish(
        &self,
        id: i64,
        status: SessionStatus,
        ended_at: &Timestamp,
        summary: Option<&Value>,
    ) -> Result<()> {
        if status == SessionStatus::Active {
            return Err(StorageError::Rule("a session cannot be finished as active"));
        }
        let result = sqlx::query(
            "UPDATE sessions SET status = ?2, ended_at = ?3, summary_json = ?4 \
             WHERE id = ?1 AND status = 'active'",
        )
        .bind(id)
        .bind(status.as_str())
        .bind(ended_at.to_string())
        .bind(opt_json_text(summary))
        .execute(self.db.writer())
        .await?;
        if result.rows_affected() == 0 {
            return match self.get(id).await? {
                Some(_) => Err(StorageError::Rule("the session has already ended")),
                None => Err(StorageError::NotFound { what: "session" }),
            };
        }
        Ok(())
    }

    /// Paths of the recordings kept for this session.
    ///
    /// The database cannot delete files. Call this, delete the files, and only
    /// then call [`Sessions::delete`], so a crash in between leaves a dangling
    /// row (harmless) rather than a recording nobody knows about.
    pub async fn audio_paths(&self, id: i64) -> Result<Vec<String>> {
        let paths = sqlx::query_scalar(
            "SELECT audio_clips.path FROM audio_clips \
             JOIN turns ON turns.id = audio_clips.turn_id \
             WHERE turns.session_id = ?1 ORDER BY audio_clips.id",
        )
        .bind(id)
        .fetch_all(self.db.reader())
        .await?;
        Ok(paths)
    }

    /// Deletes one session.
    ///
    /// Its turns (chat messages and writing drafts included), turn analysis,
    /// error events, generated content, recordings and free-speech pronunciation
    /// rows go by cascade. A trigger removes the evidence text and the pending
    /// scoring of attempts made in the session. The attempt rows stay with
    /// `session_id` set to null, so scores and estimates do not change.
    pub async fn delete(&self, id: i64) -> Result<()> {
        let result = sqlx::query("DELETE FROM sessions WHERE id = ?1")
            .bind(id)
            .execute(self.db.writer())
            .await?;
        expect_changed(&result, "session")
    }
}
