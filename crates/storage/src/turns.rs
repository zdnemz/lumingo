//! Turns: a spoken turn, a chat message or a writing draft. This is where
//! learner text lives, so everything here goes away with its session.

use serde::{Deserialize, Serialize};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use crate::db::Database;
use crate::enums::{InputMode, TurnRole};
use crate::error::Result;
use crate::row::{enum_col, expect_changed, timestamp_col};
use crate::time::Timestamp;

/// A stored turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    pub id: i64,
    pub session_id: i64,
    /// Position inside the session, starting at 1.
    pub seq: i64,
    pub role: TurnRole,
    pub input_mode: InputMode,
    pub text: String,
    /// The original speech-to-text output, kept when the learner corrected it.
    pub stt_text: Option<String>,
    pub edited_by_learner: bool,
    /// Voiced time inside the turn, from the voice-activity detector.
    pub speech_ms: Option<i64>,
    /// Silent time inside the turn, from the voice-activity detector.
    pub pause_ms: Option<i64>,
    pub word_count: Option<i64>,
    pub created_at: Timestamp,
}

/// A turn to append. Its `seq` is assigned by the repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewTurn {
    pub session_id: i64,
    pub role: TurnRole,
    pub input_mode: InputMode,
    pub text: String,
    pub stt_text: Option<String>,
    pub edited_by_learner: bool,
    pub speech_ms: Option<i64>,
    pub pause_ms: Option<i64>,
    pub word_count: Option<i64>,
    pub created_at: Timestamp,
}

impl Turn {
    fn from_row(row: &SqliteRow) -> Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            session_id: row.try_get("session_id")?,
            seq: row.try_get("seq")?,
            role: enum_col(row, "role")?,
            input_mode: enum_col(row, "input_mode")?,
            text: row.try_get("text")?,
            stt_text: row.try_get("stt_text")?,
            edited_by_learner: row.try_get("edited_by_learner")?,
            speech_ms: row.try_get("speech_ms")?,
            pause_ms: row.try_get("pause_ms")?,
            word_count: row.try_get("word_count")?,
            created_at: timestamp_col(row, "created_at")?,
        })
    }
}

/// Queries on the `turns` table.
pub struct Turns<'a> {
    db: &'a Database,
}

impl Database {
    /// Turn queries.
    pub fn turns(&self) -> Turns<'_> {
        Turns { db: self }
    }
}

impl Turns<'_> {
    /// Appends a turn to its session and returns it with its `seq`.
    ///
    /// Reading the next `seq` and inserting happen in one write transaction, so
    /// two appends can never take the same number.
    pub async fn append(&self, new: &NewTurn) -> Result<Turn> {
        let mut tx = self.db.begin_write().await?;
        let next: i64 =
            sqlx::query_scalar("SELECT COALESCE(MAX(seq), 0) + 1 FROM turns WHERE session_id = ?1")
                .bind(new.session_id)
                .fetch_one(&mut *tx)
                .await?;
        let row = sqlx::query(
            "INSERT INTO turns \
             (session_id, seq, role, input_mode, text, stt_text, edited_by_learner, \
              speech_ms, pause_ms, word_count, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) RETURNING *",
        )
        .bind(new.session_id)
        .bind(next)
        .bind(new.role.as_str())
        .bind(new.input_mode.as_str())
        .bind(&new.text)
        .bind(&new.stt_text)
        .bind(new.edited_by_learner)
        .bind(new.speech_ms)
        .bind(new.pause_ms)
        .bind(new.word_count)
        .bind(new.created_at.to_string())
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Turn::from_row(&row)
    }

    /// The turn with this id, if it exists.
    pub async fn get(&self, id: i64) -> Result<Option<Turn>> {
        let row = sqlx::query("SELECT * FROM turns WHERE id = ?1")
            .bind(id)
            .fetch_optional(self.db.reader())
            .await?;
        row.as_ref().map(Turn::from_row).transpose()
    }

    /// Every turn of a session in order.
    pub async fn list(&self, session_id: i64) -> Result<Vec<Turn>> {
        let rows = sqlx::query("SELECT * FROM turns WHERE session_id = ?1 ORDER BY seq")
            .bind(session_id)
            .fetch_all(self.db.reader())
            .await?;
        rows.iter().map(Turn::from_row).collect()
    }

    /// The last `limit` turns of a session, oldest first. The tutor's history is
    /// bounded, so it never needs more.
    pub async fn recent(&self, session_id: i64, limit: u32) -> Result<Vec<Turn>> {
        let rows = sqlx::query(
            "SELECT * FROM (SELECT * FROM turns WHERE session_id = ?1 ORDER BY seq DESC LIMIT ?2) \
             ORDER BY seq",
        )
        .bind(session_id)
        .bind(i64::from(limit))
        .fetch_all(self.db.reader())
        .await?;
        rows.iter().map(Turn::from_row).collect()
    }

    /// Replaces a turn's text after the learner corrected a transcript.
    ///
    /// The first correction keeps the original recogniser output in `stt_text`;
    /// later corrections leave it alone.
    pub async fn correct_text(&self, id: i64, text: &str, word_count: Option<i64>) -> Result<()> {
        let result = sqlx::query(
            "UPDATE turns SET stt_text = COALESCE(stt_text, text), text = ?2, \
             edited_by_learner = 1, word_count = ?3 WHERE id = ?1",
        )
        .bind(id)
        .bind(text)
        .bind(word_count)
        .execute(self.db.writer())
        .await?;
        expect_changed(&result, "turn")
    }
}
