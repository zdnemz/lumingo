//! Technical logs: one row per LLM call and per latency sample.
//!
//! These tables never hold learner text, prompts or responses. The schema has no
//! text column for them, and the free-text fields that do exist (model name,
//! metric name, hardware tag) are checked here to be short identifiers, so a
//! sentence cannot be stored in one by mistake.

use serde::{Deserialize, Serialize};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use crate::db::Database;
use crate::enums::{LlmCallType, LlmOutcome};
use crate::error::{Result, StorageError};
use crate::row::{enum_col, timestamp_col};
use crate::time::Timestamp;

const MAX_IDENTIFIER_LEN: usize = 128;

/// Accepts what model ids, metric names and hardware tags look like
/// (`gpt-4o-mini`, `meta-llama/Llama-3.1-8B`, `llm_first_sentence_ms`, `floor`)
/// and refuses anything with spaces or punctuation that prose would have.
fn check_identifier(value: &str, what: &'static str) -> Result<()> {
    let plain =
        |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '/' | '@' | '+' | '-');
    if value.is_empty() || value.len() > MAX_IDENTIFIER_LEN || !value.chars().all(plain) {
        return Err(StorageError::Rule(what));
    }
    Ok(())
}

/// A finished LLM call, as recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmCall {
    pub id: i64,
    /// Null after the provider profile was removed.
    pub provider_profile_id: Option<i64>,
    pub call_type: LlmCallType,
    pub model: String,
    pub ladder_level: Option<i64>,
    /// Milliseconds to the first streamed token.
    pub ttft_ms: Option<i64>,
    pub total_ms: Option<i64>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub http_status: Option<i64>,
    pub outcome: LlmOutcome,
    pub started_at: Timestamp,
}

/// An LLM call to record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewLlmCall {
    pub provider_profile_id: Option<i64>,
    pub call_type: LlmCallType,
    pub model: String,
    pub ladder_level: Option<i64>,
    pub ttft_ms: Option<i64>,
    pub total_ms: Option<i64>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub http_status: Option<i64>,
    pub outcome: LlmOutcome,
    pub started_at: Timestamp,
}

/// One latency measurement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerfSample {
    pub id: i64,
    /// Null after the session was deleted: the number outlives its session.
    pub session_id: Option<i64>,
    pub turn_seq: Option<i64>,
    /// For example `endpoint_ms`, `stt_ms`, `llm_first_sentence_ms`, `e2e_ms`.
    pub metric: String,
    pub value_ms: f64,
    /// Hardware profile label, for example `dev` or `floor`.
    pub profile_tag: String,
    pub created_at: Timestamp,
}

/// A latency measurement to record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewPerfSample {
    pub session_id: Option<i64>,
    pub turn_seq: Option<i64>,
    pub metric: String,
    pub value_ms: f64,
    pub profile_tag: String,
    pub created_at: Timestamp,
}

impl LlmCall {
    fn from_row(row: &SqliteRow) -> Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            provider_profile_id: row.try_get("provider_profile_id")?,
            call_type: enum_col(row, "call_type")?,
            model: row.try_get("model")?,
            ladder_level: row.try_get("ladder_level")?,
            ttft_ms: row.try_get("ttft_ms")?,
            total_ms: row.try_get("total_ms")?,
            input_tokens: row.try_get("input_tokens")?,
            output_tokens: row.try_get("output_tokens")?,
            http_status: row.try_get("http_status")?,
            outcome: enum_col(row, "outcome")?,
            started_at: timestamp_col(row, "started_at")?,
        })
    }
}

impl PerfSample {
    fn from_row(row: &SqliteRow) -> Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            session_id: row.try_get("session_id")?,
            turn_seq: row.try_get("turn_seq")?,
            metric: row.try_get("metric")?,
            value_ms: row.try_get("value_ms")?,
            profile_tag: row.try_get("profile_tag")?,
            created_at: timestamp_col(row, "created_at")?,
        })
    }
}

/// Queries on `llm_calls` and `perf_samples`.
pub struct Diagnostics<'a> {
    db: &'a Database,
}

impl Database {
    /// Technical-log queries.
    pub fn diagnostics(&self) -> Diagnostics<'_> {
        Diagnostics { db: self }
    }
}

impl Diagnostics<'_> {
    /// Records one LLM call.
    pub async fn record_llm_call(&self, new: &NewLlmCall) -> Result<LlmCall> {
        check_identifier(&new.model, "a model name is a short identifier, not text")?;
        let row = sqlx::query(
            "INSERT INTO llm_calls \
             (provider_profile_id, call_type, model, ladder_level, ttft_ms, total_ms, \
              input_tokens, output_tokens, http_status, outcome, started_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) RETURNING *",
        )
        .bind(new.provider_profile_id)
        .bind(new.call_type.as_str())
        .bind(&new.model)
        .bind(new.ladder_level)
        .bind(new.ttft_ms)
        .bind(new.total_ms)
        .bind(new.input_tokens)
        .bind(new.output_tokens)
        .bind(new.http_status)
        .bind(new.outcome.as_str())
        .bind(new.started_at.to_string())
        .fetch_one(self.db.writer())
        .await?;
        LlmCall::from_row(&row)
    }

    /// The most recent calls, newest first.
    pub async fn recent_llm_calls(&self, limit: u32) -> Result<Vec<LlmCall>> {
        let rows =
            sqlx::query("SELECT * FROM llm_calls ORDER BY started_at DESC, id DESC LIMIT ?1")
                .bind(i64::from(limit))
                .fetch_all(self.db.reader())
                .await?;
        rows.iter().map(LlmCall::from_row).collect()
    }

    /// Records one latency measurement.
    pub async fn record_perf_sample(&self, new: &NewPerfSample) -> Result<PerfSample> {
        check_identifier(&new.metric, "a metric name is a short identifier, not text")?;
        check_identifier(
            &new.profile_tag,
            "a profile tag is a short identifier, not text",
        )?;
        let row = sqlx::query(
            "INSERT INTO perf_samples (session_id, turn_seq, metric, value_ms, profile_tag, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) RETURNING *",
        )
        .bind(new.session_id)
        .bind(new.turn_seq)
        .bind(&new.metric)
        .bind(new.value_ms)
        .bind(&new.profile_tag)
        .bind(new.created_at.to_string())
        .fetch_one(self.db.writer())
        .await?;
        PerfSample::from_row(&row)
    }

    /// Samples of one metric created at or after `since`, oldest first.
    pub async fn perf_samples(&self, metric: &str, since: &Timestamp) -> Result<Vec<PerfSample>> {
        let rows = sqlx::query(
            "SELECT * FROM perf_samples WHERE metric = ?1 AND created_at >= ?2 \
             ORDER BY created_at, id",
        )
        .bind(metric)
        .bind(since.to_string())
        .fetch_all(self.db.reader())
        .await?;
        rows.iter().map(PerfSample::from_row).collect()
    }
}
