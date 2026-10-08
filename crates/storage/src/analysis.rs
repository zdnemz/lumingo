//! The LLM's structured analysis of a learner turn, and the error events found
//! in it. Both quote the learner, so both go away with the session.
//!
//! Nothing here sets or states a CEFR level. `ladder_level` is the tutor's
//! support ladder (1 to 4), not a proficiency level.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use crate::db::Database;
use crate::enums::Severity;
use crate::error::Result;
use crate::row::{enum_col, expect_changed, json_col, json_text, timestamp_col};
use crate::time::Timestamp;

/// The validated analysis of one turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnAnalysis {
    pub turn_id: i64,
    /// The analysis object, already validated against its schema in `contracts/`.
    pub analysis: Value,
    pub contract_version: String,
    /// Support-ladder step the tutor used, 1 to 4.
    pub ladder_level: i64,
    pub model: String,
    pub created_at: Timestamp,
}

impl TurnAnalysis {
    fn from_row(row: &SqliteRow) -> Result<Self> {
        Ok(Self {
            turn_id: row.try_get("turn_id")?,
            analysis: json_col(row, "analysis_json")?,
            contract_version: row.try_get("contract_version")?,
            ladder_level: row.try_get("ladder_level")?,
            model: row.try_get("model")?,
            created_at: timestamp_col(row, "created_at")?,
        })
    }
}

/// One error found in a learner turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorEvent {
    pub id: i64,
    pub turn_id: i64,
    pub profile_id: i64,
    pub category: String,
    /// The learner's words, verbatim.
    pub quote: String,
    pub correction: String,
    pub severity: Severity,
    pub addressed: bool,
    pub created_at: Timestamp,
}

/// An error event to store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewErrorEvent {
    pub turn_id: i64,
    pub profile_id: i64,
    pub category: String,
    pub quote: String,
    pub correction: String,
    pub severity: Severity,
    pub created_at: Timestamp,
}

impl ErrorEvent {
    fn from_row(row: &SqliteRow) -> Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            turn_id: row.try_get("turn_id")?,
            profile_id: row.try_get("profile_id")?,
            category: row.try_get("category")?,
            quote: row.try_get("quote")?,
            correction: row.try_get("correction")?,
            severity: enum_col(row, "severity")?,
            addressed: row.try_get("addressed")?,
            created_at: timestamp_col(row, "created_at")?,
        })
    }
}

/// Queries on `turn_analysis` and `error_events`.
pub struct Analysis<'a> {
    db: &'a Database,
}

impl Database {
    /// Turn-analysis queries.
    pub fn analysis(&self) -> Analysis<'_> {
        Analysis { db: self }
    }
}

impl Analysis<'_> {
    /// Stores the analysis of a turn together with the error events found in it,
    /// all or nothing. A second call for the same turn replaces the analysis and
    /// adds the new events.
    pub async fn store(&self, analysis: &TurnAnalysis, events: &[NewErrorEvent]) -> Result<()> {
        let mut tx = self.db.begin_write().await?;
        sqlx::query(
            "INSERT INTO turn_analysis \
             (turn_id, analysis_json, contract_version, ladder_level, model, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT (turn_id) DO UPDATE SET analysis_json = excluded.analysis_json, \
             contract_version = excluded.contract_version, ladder_level = excluded.ladder_level, \
             model = excluded.model, created_at = excluded.created_at",
        )
        .bind(analysis.turn_id)
        .bind(json_text(&analysis.analysis))
        .bind(&analysis.contract_version)
        .bind(analysis.ladder_level)
        .bind(&analysis.model)
        .bind(analysis.created_at.to_string())
        .execute(&mut *tx)
        .await?;
        for event in events {
            sqlx::query(
                "INSERT INTO error_events \
                 (turn_id, profile_id, category, quote, correction, severity, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )
            .bind(event.turn_id)
            .bind(event.profile_id)
            .bind(&event.category)
            .bind(&event.quote)
            .bind(&event.correction)
            .bind(event.severity.as_str())
            .bind(event.created_at.to_string())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// The analysis of a turn, if one was stored.
    pub async fn get(&self, turn_id: i64) -> Result<Option<TurnAnalysis>> {
        let row = sqlx::query("SELECT * FROM turn_analysis WHERE turn_id = ?1")
            .bind(turn_id)
            .fetch_optional(self.db.reader())
            .await?;
        row.as_ref().map(TurnAnalysis::from_row).transpose()
    }

    /// The error events of a turn, in the order they were stored.
    pub async fn error_events(&self, turn_id: i64) -> Result<Vec<ErrorEvent>> {
        let rows = sqlx::query("SELECT * FROM error_events WHERE turn_id = ?1 ORDER BY id")
            .bind(turn_id)
            .fetch_all(self.db.reader())
            .await?;
        rows.iter().map(ErrorEvent::from_row).collect()
    }

    /// Marks an error as addressed once the tutor has worked on it.
    pub async fn mark_addressed(&self, error_event_id: i64) -> Result<()> {
        let result = sqlx::query("UPDATE error_events SET addressed = 1 WHERE id = ?1")
            .bind(error_event_id)
            .execute(self.db.writer())
            .await?;
        expect_changed(&result, "error event")
    }
}
