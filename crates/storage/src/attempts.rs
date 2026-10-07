//! Attempts, evidence, and the pending-scoring queue.
//!
//! One submitted response is scored per dimension; the rows share a
//! `response_id` and the estimation code groups by it, so one essay is one
//! piece of evidence. Evidence rows hold the learner text and quotes that make
//! a score traceable; the trigger in the schema clears them when a session is
//! deleted while the attempt row stays.

use crate::db::Database;
use crate::error::{StorageError, classify, not_found};
use crate::models::{
    Attempt, Evidence, NewAttempt, NewEvidence, NewPendingScoring, PendingScoring, Skill,
};
use crate::rows::{AttemptRow, EvidenceRow, PendingScoringRow};

impl Database {
    /// Writes one attempt row and returns it.
    pub async fn add_attempt(&self, attempt: NewAttempt) -> Result<Attempt, StorageError> {
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO assessment_attempts \
             (profile_id, session_id, unit_id, activity_id, activity_type, response_id, \
              origin, level, skill, dimension, scorer, scorer_version, raw_score, \
              max_score, normalized, confidence, status, counts_toward_estimate, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(attempt.profile_id)
        .bind(attempt.session_id)
        .bind(&attempt.unit_id)
        .bind(&attempt.activity_id)
        .bind(&attempt.activity_type)
        .bind(&attempt.response_id)
        .bind(attempt.origin.as_str())
        .bind(attempt.level.as_str())
        .bind(attempt.skill.as_str())
        .bind(&attempt.dimension)
        .bind(attempt.scorer.as_str())
        .bind(&attempt.scorer_version)
        .bind(attempt.raw_score)
        .bind(attempt.max_score)
        .bind(attempt.normalized)
        .bind(attempt.confidence)
        .bind(attempt.status.as_str())
        .bind(attempt.counts_toward_estimate)
        .bind(&attempt.created_at)
        .fetch_one(self.writer())
        .await
        .map_err(|error| classify("assessment_attempts", error))?;

        self.attempt(id).await
    }

    /// Reads one attempt.
    pub async fn attempt(&self, id: i64) -> Result<Attempt, StorageError> {
        let row: Option<AttemptRow> = sqlx::query_as(SELECT_ATTEMPT_BY_ID)
            .bind(id)
            .fetch_optional(self.readers())
            .await?;

        row.map(Attempt::try_from)
            .transpose()?
            .ok_or_else(|| not_found("assessment_attempts", id))
    }

    /// Every attempt of one response, for example the four dimensions of one
    /// essay.
    pub async fn attempts_for_response(
        &self,
        response_id: &str,
    ) -> Result<Vec<Attempt>, StorageError> {
        let rows: Vec<AttemptRow> = sqlx::query_as(
            "SELECT id, profile_id, session_id, unit_id, activity_id, activity_type, \
                    response_id, origin, level, skill, dimension, scorer, scorer_version, \
                    raw_score, max_score, normalized, confidence, status, \
                    counts_toward_estimate, created_at \
             FROM assessment_attempts WHERE response_id = ? ORDER BY id",
        )
        .bind(response_id)
        .fetch_all(self.readers())
        .await?;

        rows.into_iter().map(Attempt::try_from).collect()
    }

    /// The attempts of one skill that the estimate function considers, newest
    /// first, as the plain rows `assessment-engine` reads. The engine applies
    /// its own eligibility rules; this query only narrows by skill and profile.
    pub async fn attempts_for_skill(
        &self,
        profile_id: i64,
        skill: Skill,
    ) -> Result<Vec<Attempt>, StorageError> {
        let rows: Vec<AttemptRow> = sqlx::query_as(
            "SELECT id, profile_id, session_id, unit_id, activity_id, activity_type, \
                    response_id, origin, level, skill, dimension, scorer, scorer_version, \
                    raw_score, max_score, normalized, confidence, status, \
                    counts_toward_estimate, created_at \
             FROM assessment_attempts \
             WHERE profile_id = ? AND skill = ? \
             ORDER BY created_at DESC, id DESC",
        )
        .bind(profile_id)
        .bind(skill.as_str())
        .fetch_all(self.readers())
        .await?;

        rows.into_iter().map(Attempt::try_from).collect()
    }

    /// Adds one evidence row.
    pub async fn add_evidence(&self, evidence: NewEvidence) -> Result<Evidence, StorageError> {
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO assessment_evidence (attempt_id, kind, content, data_json, created_at) \
             VALUES (?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(evidence.attempt_id)
        .bind(evidence.kind.as_str())
        .bind(&evidence.content)
        .bind(&evidence.data_json)
        .bind(&evidence.created_at)
        .fetch_one(self.writer())
        .await
        .map_err(|error| classify("assessment_evidence", error))?;

        self.evidence(id).await
    }

    /// Reads one evidence row.
    pub async fn evidence(&self, id: i64) -> Result<Evidence, StorageError> {
        let row: Option<EvidenceRow> = sqlx::query_as(
            "SELECT id, attempt_id, kind, content, data_json, created_at \
             FROM assessment_evidence WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.readers())
        .await?;

        row.map(Evidence::try_from)
            .transpose()?
            .ok_or_else(|| not_found("assessment_evidence", id))
    }

    /// Every evidence row of one attempt, in order.
    pub async fn evidence_for_attempt(
        &self,
        attempt_id: i64,
    ) -> Result<Vec<Evidence>, StorageError> {
        let rows: Vec<EvidenceRow> = sqlx::query_as(
            "SELECT id, attempt_id, kind, content, data_json, created_at \
             FROM assessment_evidence WHERE attempt_id = ? ORDER BY id",
        )
        .bind(attempt_id)
        .fetch_all(self.readers())
        .await?;

        rows.into_iter().map(Evidence::try_from).collect()
    }

    /// Queues a productive response until a provider is reachable. The attempt
    /// is unique, so queueing the same attempt twice is a conflict, not a
    /// duplicate.
    pub async fn enqueue_pending_scoring(
        &self,
        pending: NewPendingScoring,
    ) -> Result<PendingScoring, StorageError> {
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO pending_scoring (attempt_id, payload_json, created_at) \
             VALUES (?, ?, ?) RETURNING id",
        )
        .bind(pending.attempt_id)
        .bind(&pending.payload_json)
        .bind(&pending.created_at)
        .fetch_one(self.writer())
        .await
        .map_err(|error| classify("pending_scoring", error))?;

        let row: PendingScoringRow =
            sqlx::query_as("SELECT id, attempt_id, payload_json, tries, created_at FROM pending_scoring WHERE id = ?")
                .bind(id)
                .fetch_one(self.readers())
                .await?;

        PendingScoring::try_from(row)
    }

    /// The pending queue, oldest first, for draining when a provider returns.
    pub async fn pending_scoring(&self, limit: i64) -> Result<Vec<PendingScoring>, StorageError> {
        let rows: Vec<PendingScoringRow> = sqlx::query_as(
            "SELECT id, attempt_id, payload_json, tries, created_at \
             FROM pending_scoring ORDER BY id LIMIT ?",
        )
        .bind(limit)
        .fetch_all(self.readers())
        .await?;

        rows.into_iter().map(PendingScoring::try_from).collect()
    }

    /// Counts a failed scoring attempt on a queue row.
    pub async fn bump_pending_tries(&self, id: i64) -> Result<(), StorageError> {
        let result = sqlx::query("UPDATE pending_scoring SET tries = tries + 1 WHERE id = ?")
            .bind(id)
            .execute(self.writer())
            .await
            .map_err(|error| classify("pending_scoring", error))?;

        if result.rows_affected() == 0 {
            return Err(not_found("pending_scoring", id));
        }
        Ok(())
    }

    /// Removes a queue row once its response has been scored.
    pub async fn remove_pending_scoring(&self, id: i64) -> Result<(), StorageError> {
        let result = sqlx::query("DELETE FROM pending_scoring WHERE id = ?")
            .bind(id)
            .execute(self.writer())
            .await
            .map_err(|error| classify("pending_scoring", error))?;

        if result.rows_affected() == 0 {
            return Err(not_found("pending_scoring", id));
        }
        Ok(())
    }
}

const SELECT_ATTEMPT_BY_ID: &str = "SELECT id, profile_id, session_id, unit_id, activity_id, \
     activity_type, response_id, origin, level, skill, dimension, scorer, scorer_version, \
     raw_score, max_score, normalized, confidence, status, counts_toward_estimate, created_at \
     FROM assessment_attempts WHERE id = ?";
