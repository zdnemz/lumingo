//! Sessions: the unit of one lesson, conversation, free mode or test.
//!
//! Deleting a session removes its text (turns, analysis, error events, generated
//! content, recordings and free-speech pronunciation rows go by cascade; the
//! schema trigger clears evidence and pending scoring of attempts made in it).
//! The attempt rows themselves stay, with `session_id` set to NULL, so scores
//! and estimates are unchanged.

use crate::db::Database;
use crate::error::{StorageError, classify, not_found};
use crate::models::{AudioClip, NewSession, Session, SessionStatus};
use crate::rows::SessionRow;

impl Database {
    /// Starts a session and returns the stored row.
    pub async fn create_session(&self, session: NewSession) -> Result<Session, StorageError> {
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO sessions \
             (profile_id, kind, unit_id, activity_id, mode, provider_profile_id, \
              app_version, started_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(session.profile_id)
        .bind(session.kind.as_str())
        .bind(&session.unit_id)
        .bind(&session.activity_id)
        .bind(session.mode.map(|mode| mode.as_str()))
        .bind(session.provider_profile_id)
        .bind(&session.app_version)
        .bind(&session.started_at)
        .fetch_one(self.writer())
        .await
        .map_err(|error| classify("sessions", error))?;

        self.session(id).await
    }

    /// Reads one session.
    pub async fn session(&self, id: i64) -> Result<Session, StorageError> {
        let row: Option<SessionRow> = sqlx::query_as(
            "SELECT id, profile_id, kind, unit_id, activity_id, mode, status, \
                    provider_profile_id, summary_json, app_version, started_at, ended_at \
             FROM sessions WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.readers())
        .await?;

        row.map(Session::try_from)
            .transpose()?
            .ok_or_else(|| not_found("sessions", id))
    }

    /// A learner's sessions, newest first.
    pub async fn sessions_for_profile(
        &self,
        profile_id: i64,
        limit: i64,
    ) -> Result<Vec<Session>, StorageError> {
        let rows: Vec<SessionRow> = sqlx::query_as(
            "SELECT id, profile_id, kind, unit_id, activity_id, mode, status, \
                    provider_profile_id, summary_json, app_version, started_at, ended_at \
             FROM sessions WHERE profile_id = ? ORDER BY started_at DESC, id DESC LIMIT ?",
        )
        .bind(profile_id)
        .bind(limit)
        .fetch_all(self.readers())
        .await?;

        rows.into_iter().map(Session::try_from).collect()
    }

    /// Ends a session. Only `Completed` and `Aborted` are accepted; an active
    /// session that is simply left behind stays active.
    pub async fn end_session(
        &self,
        id: i64,
        status: SessionStatus,
        ended_at: &str,
    ) -> Result<(), StorageError> {
        if status == SessionStatus::Active {
            return Err(StorageError::Invalid {
                table: "sessions",
                detail: "end_session needs completed or aborted".to_owned(),
            });
        }

        let result = sqlx::query("UPDATE sessions SET status = ?, ended_at = ? WHERE id = ?")
            .bind(status.as_str())
            .bind(ended_at)
            .bind(id)
            .execute(self.writer())
            .await
            .map_err(|error| classify("sessions", error))?;

        if result.rows_affected() == 0 {
            return Err(not_found("sessions", id));
        }
        Ok(())
    }

    /// Writes the session summary (valid JSON; the schema checks it).
    pub async fn set_session_summary(
        &self,
        id: i64,
        summary_json: &str,
    ) -> Result<(), StorageError> {
        let result = sqlx::query("UPDATE sessions SET summary_json = ? WHERE id = ?")
            .bind(summary_json)
            .bind(id)
            .execute(self.writer())
            .await
            .map_err(|error| classify("sessions", error))?;

        if result.rows_affected() == 0 {
            return Err(not_found("sessions", id));
        }
        Ok(())
    }

    /// The recordings of one session. The caller deletes these files before
    /// calling [`delete_session`], because the database cannot delete files.
    ///
    /// [`delete_session`]: Database::delete_session
    pub async fn audio_clips_for_session(
        &self,
        session_id: i64,
    ) -> Result<Vec<AudioClip>, StorageError> {
        let rows: Vec<AudioClipRow> = sqlx::query_as(
            "SELECT a.id, a.turn_id, a.path, a.duration_ms, a.created_at \
             FROM audio_clips a JOIN turns t ON t.id = a.turn_id \
             WHERE t.session_id = ? ORDER BY a.id",
        )
        .bind(session_id)
        .fetch_all(self.readers())
        .await?;

        Ok(rows.into_iter().map(AudioClip::from).collect())
    }

    /// Every recording of a profile, for deleting all learner data.
    pub async fn audio_clips_for_profile(
        &self,
        profile_id: i64,
    ) -> Result<Vec<AudioClip>, StorageError> {
        let rows: Vec<AudioClipRow> = sqlx::query_as(
            "SELECT a.id, a.turn_id, a.path, a.duration_ms, a.created_at \
             FROM audio_clips a \
             JOIN turns t ON t.id = a.turn_id \
             JOIN sessions s ON s.id = t.session_id \
             WHERE s.profile_id = ? ORDER BY a.id",
        )
        .bind(profile_id)
        .fetch_all(self.readers())
        .await?;

        Ok(rows.into_iter().map(AudioClip::from).collect())
    }

    /// Deletes one session and everything that cascades from it. Attempts made
    /// in the session stay with `session_id` NULL; see the module docs.
    pub async fn delete_session(&self, id: i64) -> Result<(), StorageError> {
        let result = sqlx::query("DELETE FROM sessions WHERE id = ?")
            .bind(id)
            .execute(self.writer())
            .await
            .map_err(|error| classify("sessions", error))?;

        if result.rows_affected() == 0 {
            return Err(not_found("sessions", id));
        }
        Ok(())
    }
}

#[derive(sqlx::FromRow)]
struct AudioClipRow {
    id: i64,
    turn_id: i64,
    path: String,
    duration_ms: i64,
    created_at: String,
}

impl From<AudioClipRow> for AudioClip {
    fn from(row: AudioClipRow) -> Self {
        AudioClip {
            id: row.id,
            turn_id: row.turn_id,
            path: row.path,
            duration_ms: row.duration_ms,
            created_at: row.created_at,
        }
    }
}
