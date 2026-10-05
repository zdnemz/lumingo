//! Assessment data: scored attempts, the evidence behind them and the queue of
//! responses waiting for a provider.
//!
//! These are plain inserts and reads. Deciding what a score means, which
//! attempts are eligible and what level they support is the assessment crate's
//! job (`docs/ASSESSMENT_SPEC.md`); nothing here computes a score or a level.
//!
//! One response scored on several dimensions is several rows that share a
//! `response_id`. The estimation code groups by it, so one essay is one piece of
//! evidence: see [`group_by_response`].

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use crate::db::Database;
use crate::enums::{AttemptOrigin, AttemptStatus, EvidenceKind, Level, Scorer};
use crate::error::{Result, StorageError};
use crate::row::{
    enum_col, expect_changed, json_col, json_text, opt_json_col, opt_json_text, timestamp_col,
};
use crate::time::Timestamp;

/// One scored dimension of one response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attempt {
    pub id: i64,
    pub profile_id: i64,
    /// Null after the session was deleted: the scores outlive their session.
    pub session_id: Option<i64>,
    pub unit_id: Option<String>,
    pub activity_id: String,
    pub activity_type: String,
    /// One id per submitted response. Its dimension rows share it.
    pub response_id: String,
    pub origin: AttemptOrigin,
    /// Level of the activity, from authored content.
    pub level: Level,
    pub skill: String,
    pub dimension: String,
    pub scorer: Scorer,
    /// Rubric id and version, contract version, model id or engine version.
    pub scorer_version: String,
    pub raw_score: Option<f64>,
    pub max_score: Option<f64>,
    /// Score scaled to 0..=1, enforced by the schema.
    pub normalized: Option<f64>,
    pub confidence: Option<f64>,
    pub status: AttemptStatus,
    pub counts_toward_estimate: bool,
    pub created_at: Timestamp,
}

/// An attempt row to insert.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewAttempt {
    pub profile_id: i64,
    pub session_id: Option<i64>,
    pub unit_id: Option<String>,
    pub activity_id: String,
    pub activity_type: String,
    pub response_id: String,
    pub origin: AttemptOrigin,
    pub level: Level,
    pub skill: String,
    pub dimension: String,
    pub scorer: Scorer,
    pub scorer_version: String,
    pub raw_score: Option<f64>,
    pub max_score: Option<f64>,
    pub normalized: Option<f64>,
    pub confidence: Option<f64>,
    pub status: AttemptStatus,
    pub counts_toward_estimate: bool,
    pub created_at: Timestamp,
}

/// The fields that change when a response waiting for the provider is scored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoreUpdate {
    pub scorer_version: String,
    pub raw_score: Option<f64>,
    pub max_score: Option<f64>,
    pub normalized: Option<f64>,
    pub confidence: Option<f64>,
    pub status: AttemptStatus,
}

/// All rows of one response, in the order they were inserted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResponseGroup {
    pub response_id: String,
    pub attempts: Vec<Attempt>,
}

/// Groups attempts by `response_id`, keeping the order in which each response
/// first appears. Two essays scored on four dimensions each are eight rows but
/// two groups.
pub fn group_by_response(attempts: Vec<Attempt>) -> Vec<ResponseGroup> {
    let mut groups: Vec<ResponseGroup> = Vec::new();
    for attempt in attempts {
        match groups
            .iter_mut()
            .find(|group| group.response_id == attempt.response_id)
        {
            Some(group) => group.attempts.push(attempt),
            None => groups.push(ResponseGroup {
                response_id: attempt.response_id.clone(),
                attempts: vec![attempt],
            }),
        }
    }
    groups
}

impl Attempt {
    fn from_row(row: &SqliteRow) -> Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            profile_id: row.try_get("profile_id")?,
            session_id: row.try_get("session_id")?,
            unit_id: row.try_get("unit_id")?,
            activity_id: row.try_get("activity_id")?,
            activity_type: row.try_get("activity_type")?,
            response_id: row.try_get("response_id")?,
            origin: enum_col(row, "origin")?,
            level: enum_col(row, "level")?,
            skill: row.try_get("skill")?,
            dimension: row.try_get("dimension")?,
            scorer: enum_col(row, "scorer")?,
            scorer_version: row.try_get("scorer_version")?,
            raw_score: row.try_get("raw_score")?,
            max_score: row.try_get("max_score")?,
            normalized: row.try_get("normalized")?,
            confidence: row.try_get("confidence")?,
            status: enum_col(row, "status")?,
            counts_toward_estimate: row.try_get("counts_toward_estimate")?,
            created_at: timestamp_col(row, "created_at")?,
        })
    }
}

const INSERT_ATTEMPT: &str = "INSERT INTO assessment_attempts \
    (profile_id, session_id, unit_id, activity_id, activity_type, response_id, origin, level, \
     skill, dimension, scorer, scorer_version, raw_score, max_score, normalized, confidence, \
     status, counts_toward_estimate, created_at) \
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19) \
    RETURNING *";

/// Free-mode and generated work never counts toward a level estimate. The
/// estimation code filters on `origin` as well; refusing the combination here
/// means a bug elsewhere cannot store a row that contradicts it.
fn check_rules(new: &NewAttempt) -> Result<()> {
    if new.origin != AttemptOrigin::Authored && new.counts_toward_estimate {
        return Err(StorageError::Rule(
            "only authored work can count toward an estimate",
        ));
    }
    Ok(())
}

fn bind_attempt<'q>(
    query: sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments>,
    new: &'q NewAttempt,
) -> sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments> {
    query
        .bind(new.profile_id)
        .bind(new.session_id)
        .bind(&new.unit_id)
        .bind(&new.activity_id)
        .bind(&new.activity_type)
        .bind(&new.response_id)
        .bind(new.origin.as_str())
        .bind(new.level.as_str())
        .bind(&new.skill)
        .bind(&new.dimension)
        .bind(new.scorer.as_str())
        .bind(&new.scorer_version)
        .bind(new.raw_score)
        .bind(new.max_score)
        .bind(new.normalized)
        .bind(new.confidence)
        .bind(new.status.as_str())
        .bind(new.counts_toward_estimate)
        .bind(new.created_at.to_string())
}

/// Queries on the `assessment_attempts` table.
pub struct Attempts<'a> {
    db: &'a Database,
}

impl Database {
    /// Attempt queries.
    pub fn attempts(&self) -> Attempts<'_> {
        Attempts { db: self }
    }
}

impl Attempts<'_> {
    /// Inserts one attempt row.
    pub async fn insert(&self, new: &NewAttempt) -> Result<Attempt> {
        check_rules(new)?;
        let row = bind_attempt(sqlx::query(INSERT_ATTEMPT), new)
            .fetch_one(self.db.writer())
            .await?;
        Attempt::from_row(&row)
    }

    /// Inserts every dimension row of one response, all or nothing.
    ///
    /// Refuses an empty list, rows with different `response_id`s and rows of
    /// different profiles, so a response can never be split or mixed.
    pub async fn insert_response(&self, rows: &[NewAttempt]) -> Result<Vec<Attempt>> {
        let Some(first) = rows.first() else {
            return Err(StorageError::Rule("a response has at least one dimension"));
        };
        if rows
            .iter()
            .any(|r| r.response_id != first.response_id || r.profile_id != first.profile_id)
        {
            return Err(StorageError::Rule(
                "all rows of a response share one response_id and one profile",
            ));
        }
        for new in rows {
            check_rules(new)?;
        }
        let mut tx = self.db.begin_write().await?;
        let mut stored = Vec::with_capacity(rows.len());
        for new in rows {
            let row = bind_attempt(sqlx::query(INSERT_ATTEMPT), new)
                .fetch_one(&mut *tx)
                .await?;
            stored.push(Attempt::from_row(&row)?);
        }
        tx.commit().await?;
        Ok(stored)
    }

    /// The attempt with this id, if it exists.
    pub async fn get(&self, id: i64) -> Result<Option<Attempt>> {
        let row = sqlx::query("SELECT * FROM assessment_attempts WHERE id = ?1")
            .bind(id)
            .fetch_optional(self.db.reader())
            .await?;
        row.as_ref().map(Attempt::from_row).transpose()
    }

    /// Every dimension row of one response, in insertion order.
    pub async fn by_response(&self, response_id: &str) -> Result<Vec<Attempt>> {
        let rows =
            sqlx::query("SELECT * FROM assessment_attempts WHERE response_id = ?1 ORDER BY id")
                .bind(response_id)
                .fetch_all(self.db.reader())
                .await?;
        rows.iter().map(Attempt::from_row).collect()
    }

    /// The attempts made in a session. A deleted session has none left.
    pub async fn for_session(&self, session_id: i64) -> Result<Vec<Attempt>> {
        let rows =
            sqlx::query("SELECT * FROM assessment_attempts WHERE session_id = ?1 ORDER BY id")
                .bind(session_id)
                .fetch_all(self.db.reader())
                .await?;
        rows.iter().map(Attempt::from_row).collect()
    }

    /// Every attempt of one skill created at or after `since`, oldest first.
    ///
    /// This is the one read the estimation code needs per skill
    /// (`docs/ASSESSMENT_SPEC.md` 9.1). It returns every row, eligible or not,
    /// because whether a response is eligible depends on all of its rows: the
    /// caller groups with [`group_by_response`] and applies the rule.
    pub async fn for_skill_since(
        &self,
        profile_id: i64,
        skill: &str,
        since: &Timestamp,
    ) -> Result<Vec<Attempt>> {
        let rows = sqlx::query(
            "SELECT * FROM assessment_attempts \
             WHERE profile_id = ?1 AND skill = ?2 AND created_at >= ?3 \
             ORDER BY created_at, id",
        )
        .bind(profile_id)
        .bind(skill)
        .bind(since.to_string())
        .fetch_all(self.db.reader())
        .await?;
        rows.iter().map(Attempt::from_row).collect()
    }

    /// Writes the result of a score that was waiting for the provider.
    pub async fn update_score(&self, id: i64, update: &ScoreUpdate) -> Result<()> {
        let result = sqlx::query(
            "UPDATE assessment_attempts SET scorer_version = ?2, raw_score = ?3, max_score = ?4, \
             normalized = ?5, confidence = ?6, status = ?7 WHERE id = ?1",
        )
        .bind(id)
        .bind(&update.scorer_version)
        .bind(update.raw_score)
        .bind(update.max_score)
        .bind(update.normalized)
        .bind(update.confidence)
        .bind(update.status.as_str())
        .execute(self.db.writer())
        .await?;
        expect_changed(&result, "attempt")
    }
}

/// Something that backs a score: the response text, a verbatim quote, a metric
/// or the scorer's reason. Text kinds hold learner words and are deleted with
/// the session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    pub id: i64,
    pub attempt_id: i64,
    pub kind: EvidenceKind,
    pub content: Option<String>,
    pub data: Option<Value>,
    pub created_at: Timestamp,
}

/// Evidence to store.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewEvidence {
    pub attempt_id: i64,
    pub kind: EvidenceKind,
    pub content: Option<String>,
    pub data: Option<Value>,
    pub created_at: Timestamp,
}

impl Evidence {
    fn from_row(row: &SqliteRow) -> Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            attempt_id: row.try_get("attempt_id")?,
            kind: enum_col(row, "kind")?,
            content: row.try_get("content")?,
            data: opt_json_col(row, "data_json")?,
            created_at: timestamp_col(row, "created_at")?,
        })
    }
}

/// Queries on the `assessment_evidence` table.
pub struct EvidenceRepo<'a> {
    db: &'a Database,
}

impl Database {
    /// Evidence queries.
    pub fn evidence(&self) -> EvidenceRepo<'_> {
        EvidenceRepo { db: self }
    }
}

impl EvidenceRepo<'_> {
    /// Stores one piece of evidence.
    pub async fn add(&self, new: &NewEvidence) -> Result<Evidence> {
        let row = sqlx::query(
            "INSERT INTO assessment_evidence (attempt_id, kind, content, data_json, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5) RETURNING *",
        )
        .bind(new.attempt_id)
        .bind(new.kind.as_str())
        .bind(&new.content)
        .bind(opt_json_text(new.data.as_ref()))
        .bind(new.created_at.to_string())
        .fetch_one(self.db.writer())
        .await?;
        Evidence::from_row(&row)
    }

    /// The evidence of one attempt, in insertion order.
    pub async fn for_attempt(&self, attempt_id: i64) -> Result<Vec<Evidence>> {
        let rows =
            sqlx::query("SELECT * FROM assessment_evidence WHERE attempt_id = ?1 ORDER BY id")
                .bind(attempt_id)
                .fetch_all(self.db.reader())
                .await?;
        rows.iter().map(Evidence::from_row).collect()
    }

    /// The evidence of every dimension of one response.
    pub async fn for_response(&self, response_id: &str) -> Result<Vec<Evidence>> {
        let rows = sqlx::query(
            "SELECT assessment_evidence.* FROM assessment_evidence \
             JOIN assessment_attempts ON assessment_attempts.id = assessment_evidence.attempt_id \
             WHERE assessment_attempts.response_id = ?1 ORDER BY assessment_evidence.id",
        )
        .bind(response_id)
        .fetch_all(self.db.reader())
        .await?;
        rows.iter().map(Evidence::from_row).collect()
    }
}

/// A productive response written while no provider was reachable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingScoring {
    pub id: i64,
    pub attempt_id: i64,
    /// Everything the scorer needs, including the learner's text.
    pub payload: Value,
    pub tries: i64,
    pub created_at: Timestamp,
}

impl PendingScoring {
    fn from_row(row: &SqliteRow) -> Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            attempt_id: row.try_get("attempt_id")?,
            payload: json_col(row, "payload_json")?,
            tries: row.try_get("tries")?,
            created_at: timestamp_col(row, "created_at")?,
        })
    }
}

/// Queries on the `pending_scoring` table.
pub struct PendingScoringRepo<'a> {
    db: &'a Database,
}

impl Database {
    /// Queue of responses waiting for a provider.
    pub fn pending_scoring(&self) -> PendingScoringRepo<'_> {
        PendingScoringRepo { db: self }
    }
}

impl PendingScoringRepo<'_> {
    /// Queues an attempt for scoring. One entry per attempt.
    pub async fn enqueue(
        &self,
        attempt_id: i64,
        payload: &Value,
        created_at: &Timestamp,
    ) -> Result<PendingScoring> {
        let row = sqlx::query(
            "INSERT INTO pending_scoring (attempt_id, payload_json, created_at) \
             VALUES (?1, ?2, ?3) RETURNING *",
        )
        .bind(attempt_id)
        .bind(json_text(payload))
        .bind(created_at.to_string())
        .fetch_one(self.db.writer())
        .await?;
        PendingScoring::from_row(&row)
    }

    /// The oldest waiting entries.
    pub async fn oldest(&self, limit: u32) -> Result<Vec<PendingScoring>> {
        let rows = sqlx::query("SELECT * FROM pending_scoring ORDER BY created_at, id LIMIT ?1")
            .bind(i64::from(limit))
            .fetch_all(self.db.reader())
            .await?;
        rows.iter().map(PendingScoring::from_row).collect()
    }

    /// Counts one more failed try.
    pub async fn record_try(&self, id: i64) -> Result<()> {
        let result = sqlx::query("UPDATE pending_scoring SET tries = tries + 1 WHERE id = ?1")
            .bind(id)
            .execute(self.db.writer())
            .await?;
        expect_changed(&result, "pending scoring entry")
    }

    /// Removes an entry once its attempt has been scored.
    pub async fn remove(&self, id: i64) -> Result<()> {
        let result = sqlx::query("DELETE FROM pending_scoring WHERE id = ?1")
            .bind(id)
            .execute(self.db.writer())
            .await?;
        expect_changed(&result, "pending scoring entry")
    }
}
