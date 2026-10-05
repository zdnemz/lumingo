//! Per-phoneme pronunciation results. Pronunciation feedback is experimental, so
//! these rows are numbers and symbols for display; they never feed an estimate.
//!
//! A drill result belongs to an attempt and stays when the session is deleted. A
//! free-speech result belongs to a turn and goes with it.

use serde::{Deserialize, Serialize};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use crate::db::Database;
use crate::enums::PronMode;
use crate::error::{Result, StorageError};
use crate::row::enum_col;

/// One scored phoneme of one word.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PronResult {
    pub id: i64,
    pub attempt_id: Option<i64>,
    pub turn_id: Option<i64>,
    pub mode: PronMode,
    pub word: String,
    pub word_index: i64,
    pub phone_expected: String,
    pub phone_heard: Option<String>,
    /// Goodness of pronunciation score from the alignment engine.
    pub gop: f64,
    pub flagged: bool,
    pub start_ms: i64,
    pub end_ms: i64,
    pub engine_version: String,
}

/// A pronunciation result to store. At least one of `attempt_id` and `turn_id`
/// is set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewPronResult {
    pub attempt_id: Option<i64>,
    pub turn_id: Option<i64>,
    pub mode: PronMode,
    pub word: String,
    pub word_index: i64,
    pub phone_expected: String,
    pub phone_heard: Option<String>,
    pub gop: f64,
    pub flagged: bool,
    pub start_ms: i64,
    pub end_ms: i64,
    pub engine_version: String,
}

impl PronResult {
    fn from_row(row: &SqliteRow) -> Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            attempt_id: row.try_get("attempt_id")?,
            turn_id: row.try_get("turn_id")?,
            mode: enum_col(row, "mode")?,
            word: row.try_get("word")?,
            word_index: row.try_get("word_index")?,
            phone_expected: row.try_get("phone_expected")?,
            phone_heard: row.try_get("phone_heard")?,
            gop: row.try_get("gop")?,
            flagged: row.try_get("flagged")?,
            start_ms: row.try_get("start_ms")?,
            end_ms: row.try_get("end_ms")?,
            engine_version: row.try_get("engine_version")?,
        })
    }
}

/// Queries on the `pron_results` table.
pub struct PronResults<'a> {
    db: &'a Database,
}

impl Database {
    /// Pronunciation result queries.
    pub fn pron_results(&self) -> PronResults<'_> {
        PronResults { db: self }
    }
}

impl PronResults<'_> {
    /// Stores the results of one scored utterance, all or nothing.
    pub async fn add_many(&self, results: &[NewPronResult]) -> Result<()> {
        if results
            .iter()
            .any(|r| r.attempt_id.is_none() && r.turn_id.is_none())
        {
            return Err(StorageError::Rule(
                "a pronunciation result belongs to an attempt or a turn",
            ));
        }
        let mut tx = self.db.begin_write().await?;
        for result in results {
            sqlx::query(
                "INSERT INTO pron_results \
                 (attempt_id, turn_id, mode, word, word_index, phone_expected, phone_heard, gop, \
                  flagged, start_ms, end_ms, engine_version) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            )
            .bind(result.attempt_id)
            .bind(result.turn_id)
            .bind(result.mode.as_str())
            .bind(&result.word)
            .bind(result.word_index)
            .bind(&result.phone_expected)
            .bind(&result.phone_heard)
            .bind(result.gop)
            .bind(result.flagged)
            .bind(result.start_ms)
            .bind(result.end_ms)
            .bind(&result.engine_version)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// The results of one attempt, in word order.
    pub async fn for_attempt(&self, attempt_id: i64) -> Result<Vec<PronResult>> {
        let rows = sqlx::query(
            "SELECT * FROM pron_results WHERE attempt_id = ?1 ORDER BY word_index, start_ms, id",
        )
        .bind(attempt_id)
        .fetch_all(self.db.reader())
        .await?;
        rows.iter().map(PronResult::from_row).collect()
    }

    /// The results of one turn, in word order.
    pub async fn for_turn(&self, turn_id: i64) -> Result<Vec<PronResult>> {
        let rows = sqlx::query(
            "SELECT * FROM pron_results WHERE turn_id = ?1 ORDER BY word_index, start_ms, id",
        )
        .bind(turn_id)
        .fetch_all(self.db.reader())
        .await?;
        rows.iter().map(PronResult::from_row).collect()
    }
}
