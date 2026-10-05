//! Content tied to a session that is not a turn: text the LLM generated for it
//! (graded reading passages, extra practice items) and opt-in recordings. Both
//! are removed with the session.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use crate::db::Database;
use crate::enums::GeneratedKind;
use crate::error::Result;
use crate::row::{enum_col, json_col, json_text, timestamp_col};
use crate::time::Timestamp;

/// Text the LLM generated for a session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeneratedContent {
    pub id: i64,
    pub session_id: i64,
    pub kind: GeneratedKind,
    pub content: Value,
    pub contract_version: String,
    /// The model that produced it, so every generated item can be traced.
    pub model: String,
    pub created_at: Timestamp,
}

/// Generated content to store.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewGeneratedContent {
    pub session_id: i64,
    pub kind: GeneratedKind,
    pub content: Value,
    pub contract_version: String,
    pub model: String,
    pub created_at: Timestamp,
}

impl GeneratedContent {
    fn from_row(row: &SqliteRow) -> Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            session_id: row.try_get("session_id")?,
            kind: enum_col(row, "kind")?,
            content: json_col(row, "content_json")?,
            contract_version: row.try_get("contract_version")?,
            model: row.try_get("model")?,
            created_at: timestamp_col(row, "created_at")?,
        })
    }
}

/// Queries on the `generated_content` table.
pub struct GeneratedContentRepo<'a> {
    db: &'a Database,
}

/// A recording kept with the learner's consent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioClip {
    pub id: i64,
    pub turn_id: i64,
    /// File path inside the app data directory.
    pub path: String,
    pub duration_ms: i64,
    pub created_at: Timestamp,
}

impl AudioClip {
    fn from_row(row: &SqliteRow) -> Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            turn_id: row.try_get("turn_id")?,
            path: row.try_get("path")?,
            duration_ms: row.try_get("duration_ms")?,
            created_at: timestamp_col(row, "created_at")?,
        })
    }
}

/// Queries on the `audio_clips` table.
pub struct AudioClips<'a> {
    db: &'a Database,
}

impl Database {
    /// Generated-content queries.
    pub fn generated_content(&self) -> GeneratedContentRepo<'_> {
        GeneratedContentRepo { db: self }
    }

    /// Recording-index queries.
    pub fn audio_clips(&self) -> AudioClips<'_> {
        AudioClips { db: self }
    }
}

impl GeneratedContentRepo<'_> {
    /// Stores generated content.
    pub async fn add(&self, new: &NewGeneratedContent) -> Result<GeneratedContent> {
        let row = sqlx::query(
            "INSERT INTO generated_content \
             (session_id, kind, content_json, contract_version, model, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) RETURNING *",
        )
        .bind(new.session_id)
        .bind(new.kind.as_str())
        .bind(json_text(&new.content))
        .bind(&new.contract_version)
        .bind(&new.model)
        .bind(new.created_at.to_string())
        .fetch_one(self.db.writer())
        .await?;
        GeneratedContent::from_row(&row)
    }

    /// Everything generated for a session, oldest first.
    pub async fn for_session(&self, session_id: i64) -> Result<Vec<GeneratedContent>> {
        let rows = sqlx::query("SELECT * FROM generated_content WHERE session_id = ?1 ORDER BY id")
            .bind(session_id)
            .fetch_all(self.db.reader())
            .await?;
        rows.iter().map(GeneratedContent::from_row).collect()
    }
}

impl AudioClips<'_> {
    /// Records a kept recording for a turn.
    pub async fn add(
        &self,
        turn_id: i64,
        path: &str,
        duration_ms: i64,
        created_at: &Timestamp,
    ) -> Result<AudioClip> {
        let row = sqlx::query(
            "INSERT INTO audio_clips (turn_id, path, duration_ms, created_at) \
             VALUES (?1, ?2, ?3, ?4) RETURNING *",
        )
        .bind(turn_id)
        .bind(path)
        .bind(duration_ms)
        .bind(created_at.to_string())
        .fetch_one(self.db.writer())
        .await?;
        AudioClip::from_row(&row)
    }

    /// The recordings of one turn.
    pub async fn for_turn(&self, turn_id: i64) -> Result<Vec<AudioClip>> {
        let rows = sqlx::query("SELECT * FROM audio_clips WHERE turn_id = ?1 ORDER BY id")
            .bind(turn_id)
            .fetch_all(self.db.reader())
            .await?;
        rows.iter().map(AudioClip::from_row).collect()
    }

    /// Paths of every recording a profile owns. Call it before deleting the
    /// profile, delete the files, then delete the profile.
    pub async fn paths_for_profile(&self, profile_id: i64) -> Result<Vec<String>> {
        let paths = sqlx::query_scalar(
            "SELECT audio_clips.path FROM audio_clips \
             JOIN turns ON turns.id = audio_clips.turn_id \
             JOIN sessions ON sessions.id = turns.session_id \
             WHERE sessions.profile_id = ?1 ORDER BY audio_clips.id",
        )
        .bind(profile_id)
        .fetch_all(self.db.reader())
        .await?;
        Ok(paths)
    }
}
