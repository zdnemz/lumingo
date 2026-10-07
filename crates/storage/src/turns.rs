//! Turns: spoken turns, chat messages, and writing drafts, in one table.
//!
//! `seq` is per session and unique; the learner's correction of a transcript
//! keeps the original STT text in `stt_text`.

use crate::db::Database;
use crate::error::{StorageError, classify, not_found};
use crate::models::{NewTurn, Turn};
use crate::rows::TurnRow;

impl Database {
    /// Adds a turn. The unique (session_id, seq) constraint refuses a duplicate
    /// sequence number with [`StorageError::Conflict`].
    pub async fn add_turn(&self, turn: NewTurn) -> Result<Turn, StorageError> {
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO turns \
             (session_id, seq, role, input_mode, text, stt_text, edited_by_learner, \
              speech_ms, pause_ms, word_count, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(turn.session_id)
        .bind(turn.seq)
        .bind(turn.role.as_str())
        .bind(turn.input_mode.as_str())
        .bind(&turn.text)
        .bind(&turn.stt_text)
        .bind(turn.edited_by_learner)
        .bind(turn.speech_ms)
        .bind(turn.pause_ms)
        .bind(turn.word_count)
        .bind(&turn.created_at)
        .fetch_one(self.writer())
        .await
        .map_err(|error| classify("turns", error))?;

        self.turn(id).await
    }

    /// Reads one turn.
    pub async fn turn(&self, id: i64) -> Result<Turn, StorageError> {
        let row: Option<TurnRow> = sqlx::query_as(
            "SELECT id, session_id, seq, role, input_mode, text, stt_text, \
                    edited_by_learner, speech_ms, pause_ms, word_count, created_at \
             FROM turns WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.readers())
        .await?;

        row.map(Turn::try_from)
            .transpose()?
            .ok_or_else(|| not_found("turns", id))
    }

    /// A session's turns in order.
    pub async fn turns_for_session(&self, session_id: i64) -> Result<Vec<Turn>, StorageError> {
        let rows: Vec<TurnRow> = sqlx::query_as(
            "SELECT id, session_id, seq, role, input_mode, text, stt_text, \
                    edited_by_learner, speech_ms, pause_ms, word_count, created_at \
             FROM turns WHERE session_id = ? ORDER BY seq",
        )
        .bind(session_id)
        .fetch_all(self.readers())
        .await?;

        rows.into_iter().map(Turn::try_from).collect()
    }

    /// The next free sequence number in a session.
    pub async fn next_turn_seq(&self, session_id: i64) -> Result<i64, StorageError> {
        let next: Option<i64> =
            sqlx::query_scalar("SELECT MAX(seq) + 1 FROM turns WHERE session_id = ?")
                .bind(session_id)
                .fetch_one(self.readers())
                .await?;
        Ok(next.unwrap_or(1))
    }

    /// Records the learner's correction of a transcript. The original STT text
    /// is kept when it is not already stored, so the edit is auditable.
    pub async fn edit_turn_text(&self, id: i64, text: &str) -> Result<(), StorageError> {
        let result = sqlx::query(
            "UPDATE turns \
             SET text = ?, \
                 stt_text = COALESCE(stt_text, text), \
                 edited_by_learner = 1 \
             WHERE id = ?",
        )
        .bind(text)
        .bind(id)
        .execute(self.writer())
        .await
        .map_err(|error| classify("turns", error))?;

        if result.rows_affected() == 0 {
            return Err(not_found("turns", id));
        }
        Ok(())
    }
}
