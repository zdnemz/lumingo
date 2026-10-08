//! Progress read models over storage. Read-only: nothing here writes, and
//! nothing derives a level, a score or a mastery value.

use crate::api::{
    AttemptEvidence, ErrorStatView, EvidenceView, ObjectiveMasteryView, ProgressOverview,
    ReviewDueView, SessionSummary, SkillEstimateView, UnitProgressView,
};
use crate::core::AppCore;
use crate::error::{CoreError, CoreResult};

/// How many review items the overview lists.
const REVIEW_DUE_LIMIT: u32 = 50;
/// How many recent sessions the overview lists.
const RECENT_SESSIONS: u32 = 50;

impl AppCore {
    /// The learner's progress: units, mastery, error counts, due reviews, the
    /// newest estimate of each skill and recent sessions.
    pub async fn progress(&self) -> CoreResult<ProgressOverview> {
        let profile = self.profile_id();
        let now = self.clock().now();
        let db = &self.db;

        let units = db
            .unit_progress()
            .list(profile)
            .await?
            .into_iter()
            .map(|row| UnitProgressView {
                unit_id: row.unit_id,
                status: row.status.into(),
                best_checkpoint: row.best_checkpoint,
                updated_at: row.updated_at.to_string(),
            })
            .collect();
        let objectives = db
            .mastery()
            .list(profile)
            .await?
            .into_iter()
            .map(|row| ObjectiveMasteryView {
                objective_id: row.objective_id,
                mastery: row.mastery,
                attempts: row.attempts,
                last_attempt_at: row.last_attempt_at.map(|t| t.to_string()),
            })
            .collect();
        let errors = db
            .error_stats()
            .list(profile)
            .await?
            .into_iter()
            .map(|row| ErrorStatView {
                category: row.category,
                count: row.count,
                last_seen: row.last_seen.map(|t| t.to_string()),
            })
            .collect();

        // One more than the limit tells whether the list was cut.
        let mut due = db
            .review_schedule()
            .due(profile, &now, REVIEW_DUE_LIMIT + 1)
            .await?;
        let reviews_due_truncated = due.len() > usize::try_from(REVIEW_DUE_LIMIT).unwrap_or(0);
        due.truncate(usize::try_from(REVIEW_DUE_LIMIT).unwrap_or(0));
        let reviews_due = due
            .into_iter()
            .map(|row| ReviewDueView {
                item_kind: row.item_kind.into(),
                item_ref: row.item_ref,
                due_at: row.due_at.to_string(),
                interval_days: row.interval_days,
                reps: row.reps,
                lapses: row.lapses,
            })
            .collect();

        let estimates = db
            .estimates()
            .latest_per_skill(profile)
            .await?
            .into_iter()
            .map(|row| SkillEstimateView {
                skill: row.skill,
                level: row.level.map(Into::into),
                status: row.status.into(),
                confidence: row.confidence,
                evidence_count: row.evidence_count,
                algorithm_version: row.algorithm_version,
                computed_at: row.computed_at.to_string(),
            })
            .collect();
        let recent_sessions = db
            .sessions()
            .list_for_profile(profile, RECENT_SESSIONS)
            .await?
            .into_iter()
            .map(|row| SessionSummary {
                id: row.id,
                kind: row.kind.into(),
                unit_id: row.unit_id,
                status: row.status.into(),
                started_at: row.started_at.to_string(),
                ended_at: row.ended_at.map(|t| t.to_string()),
            })
            .collect();

        Ok(ProgressOverview {
            units,
            objectives,
            errors,
            reviews_due,
            reviews_due_truncated,
            estimates,
            recent_sessions,
        })
    }

    /// What backs one scored attempt: the response, quotes, metrics and the
    /// scorer's reason. Text kinds are gone once the session is deleted.
    pub async fn attempt_evidence(&self, attempt_id: i64) -> CoreResult<AttemptEvidence> {
        let attempt = self
            .db
            .attempts()
            .get(attempt_id)
            .await?
            .ok_or(CoreError::NotFound { what: "attempt" })?;
        // An attempt of another profile is not shown. Version 1 has one profile,
        // but the check keeps the rule in place for when there are more.
        if attempt.profile_id != self.profile_id() {
            return Err(CoreError::NotFound { what: "attempt" });
        }
        let evidence = self
            .db
            .evidence()
            .for_attempt(attempt_id)
            .await?
            .into_iter()
            .map(|row| EvidenceView {
                id: row.id,
                kind: row.kind.into(),
                content: row.content,
                data: row.data,
                created_at: row.created_at.to_string(),
            })
            .collect();
        Ok(AttemptEvidence {
            attempt_id,
            activity_id: attempt.activity_id,
            skill: attempt.skill,
            dimension: attempt.dimension,
            evidence,
        })
    }
}
