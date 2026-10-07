//! Turn analysis: one row per analysed turn, holding the contract-validated
//! JSON. The JSON must already have passed its schema check in `llm-client`;
//! this table only enforces that it is valid JSON at all.
//!
//! Writing an analysis twice for one turn replaces the previous row: the turn is
//! the primary key, because a re-run analysis supersedes the first.

use crate::db::Database;
use crate::error::{StorageError, classify, not_found};
use crate::models::TurnAnalysis;

impl Database {
    /// Stores the analysis of a turn, replacing any earlier analysis of it.
    pub async fn save_turn_analysis(&self, analysis: &TurnAnalysis) -> Result<(), StorageError> {
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
        .execute(self.writer())
        .await
        .map_err(|error| classify("turn_analysis", error))?;
        Ok(())
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
