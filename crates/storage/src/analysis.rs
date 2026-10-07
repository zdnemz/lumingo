//! Turn analysis: one row per analysed turn, holding the contract-validated
//! JSON, plus the error events extracted from it and the per-category tally.
//! The JSON must already have passed its schema check in `llm-client`; this
//! table only enforces that it is valid JSON at all.
//!
//! Writing an analysis twice for one turn replaces the previous row: the turn is
//! the primary key, because a re-run analysis supersedes the first. The bundle
//! write replaces the turn's error events in the same transaction and adjusts
//! `error_stats` by the difference, so a re-analysis never double counts.

use crate::db::Database;
use crate::error::{StorageError, classify, not_found};
use crate::models::{ErrorEvent, ErrorStat, NewErrorEvent, TurnAnalysis};

impl Database {
    /// Stores the analysis of a turn, replacing any earlier analysis of it.
    pub async fn save_turn_analysis(&self, analysis: &TurnAnalysis) -> Result<(), StorageError> {
        let mut tx = self.writer().begin().await?;
        upsert_analysis(&mut tx, analysis).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Stores one analysis with its error events and updates the per-category
    /// tally, all in one transaction. Re-analysing a turn replaces its events
    /// and corrects the tally by the difference.
    pub async fn save_analysis_bundle(
        &self,
        analysis: &TurnAnalysis,
        events: &[NewErrorEvent],
    ) -> Result<(), StorageError> {
        let mut tx = self.writer().begin().await?;
        upsert_analysis(&mut tx, analysis).await?;

        // Take back what the previous analysis of this turn contributed, one
        // decrement per old event row.
        let old: Vec<(i64, String)> =
            sqlx::query_as("SELECT profile_id, category FROM error_events WHERE turn_id = ?")
                .bind(analysis.turn_id)
                .fetch_all(&mut *tx)
                .await?;
        sqlx::query("DELETE FROM error_events WHERE turn_id = ?")
            .bind(analysis.turn_id)
            .execute(&mut *tx)
            .await?;
        for (profile_id, category) in &old {
            sqlx::query(
                "UPDATE error_stats SET count = count - 1 \
                 WHERE profile_id = ? AND category = ?",
            )
            .bind(profile_id)
            .bind(category)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query("DELETE FROM error_stats WHERE count <= 0")
            .execute(&mut *tx)
            .await?;

        for event in events {
            sqlx::query(
                "INSERT INTO error_events \
                 (turn_id, profile_id, category, quote, correction, severity, addressed, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(event.turn_id)
            .bind(event.profile_id)
            .bind(&event.category)
            .bind(&event.quote)
            .bind(&event.correction)
            .bind(event.severity.as_str())
            .bind(event.addressed)
            .bind(&event.created_at)
            .execute(&mut *tx)
            .await
            .map_err(|error| classify("error_events", error))?;

            sqlx::query(
                "INSERT INTO error_stats (profile_id, category, count, last_seen) \
                 VALUES (?, ?, 1, ?) \
                 ON CONFLICT (profile_id, category) DO UPDATE SET \
                   count = error_stats.count + 1, \
                   last_seen = CASE \
                     WHEN error_stats.last_seen IS NULL \
                       OR error_stats.last_seen < excluded.last_seen \
                     THEN excluded.last_seen ELSE error_stats.last_seen END",
            )
            .bind(event.profile_id)
            .bind(&event.category)
            .bind(&event.created_at)
            .execute(&mut *tx)
            .await
            .map_err(|error| classify("error_stats", error))?;
        }

        tx.commit().await?;
        Ok(())
    }

    /// The error events of one turn, oldest first.
    pub async fn error_events_for_turn(
        &self,
        turn_id: i64,
    ) -> Result<Vec<ErrorEvent>, StorageError> {
        let rows: Vec<ErrorEventRow> = sqlx::query_as(
            "SELECT id, turn_id, profile_id, category, quote, correction, severity, addressed, created_at \
             FROM error_events WHERE turn_id = ? ORDER BY id",
        )
        .bind(turn_id)
        .fetch_all(self.readers())
        .await?;
        rows.into_iter().map(ErrorEvent::try_from).collect()
    }

    /// The per-category tally of one profile, biggest first.
    pub async fn error_stats(&self, profile_id: i64) -> Result<Vec<ErrorStat>, StorageError> {
        let rows: Vec<ErrorStatRow> = sqlx::query_as(
            "SELECT profile_id, category, count, last_seen FROM error_stats \
             WHERE profile_id = ? ORDER BY count DESC, category",
        )
        .bind(profile_id)
        .fetch_all(self.readers())
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| ErrorStat {
                profile_id: row.profile_id,
                category: row.category,
                count: row.count,
                last_seen: row.last_seen,
            })
            .collect())
    }

    /// Reads the analysis of one turn.
    pub async fn turn_analysis(&self, turn_id: i64) -> Result<TurnAnalysis, StorageError> {
        let row: Option<TurnAnalysisRow> = sqlx::query_as(
            "SELECT turn_id, analysis_json, contract_version, ladder_level, model, created_at \
             FROM turn_analysis WHERE turn_id = ?",
        )
        .bind(turn_id)
        .fetch_optional(self.readers())
        .await?;

        row.map(TurnAnalysis::from)
            .ok_or_else(|| not_found("turn_analysis", turn_id))
    }
}

async fn upsert_analysis(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    analysis: &TurnAnalysis,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO turn_analysis \
         (turn_id, analysis_json, contract_version, ladder_level, model, created_at) \
         VALUES (?, ?, ?, ?, ?, ?) \
         ON CONFLICT (turn_id) DO UPDATE SET \
           analysis_json = excluded.analysis_json, \
           contract_version = excluded.contract_version, \
           ladder_level = excluded.ladder_level, \
           model = excluded.model, \
           created_at = excluded.created_at",
    )
    .bind(analysis.turn_id)
    .bind(&analysis.analysis_json)
    .bind(&analysis.contract_version)
    .bind(analysis.ladder_level)
    .bind(&analysis.model)
    .bind(&analysis.created_at)
    .execute(&mut **tx)
    .await
    .map_err(|error| classify("turn_analysis", error))?;
    Ok(())
}

#[derive(sqlx::FromRow)]
struct TurnAnalysisRow {
    turn_id: i64,
    analysis_json: String,
    contract_version: String,
    ladder_level: i64,
    model: String,
    created_at: String,
}

impl From<TurnAnalysisRow> for TurnAnalysis {
    fn from(row: TurnAnalysisRow) -> Self {
        TurnAnalysis {
            turn_id: row.turn_id,
            analysis_json: row.analysis_json,
            contract_version: row.contract_version,
            ladder_level: row.ladder_level,
            model: row.model,
            created_at: row.created_at,
        }
    }
}

#[derive(sqlx::FromRow)]
struct ErrorEventRow {
    id: i64,
    turn_id: i64,
    profile_id: i64,
    category: String,
    quote: String,
    correction: String,
    severity: String,
    addressed: bool,
    created_at: String,
}

impl TryFrom<ErrorEventRow> for ErrorEvent {
    type Error = StorageError;

    fn try_from(row: ErrorEventRow) -> Result<Self, Self::Error> {
        Ok(ErrorEvent {
            id: row.id,
            turn_id: row.turn_id,
            profile_id: row.profile_id,
            category: row.category,
            quote: row.quote,
            correction: row.correction,
            severity: crate::rows::parse_enum("error_events", row.severity)?,
            addressed: row.addressed,
            created_at: row.created_at,
        })
    }
}

#[derive(sqlx::FromRow)]
struct ErrorStatRow {
    profile_id: i64,
    category: String,
    count: i64,
    last_seen: Option<String>,
}
