//! The estimate recompute: the production caller of the estimation rule.
//!
//! `assessment-engine` owns algorithm `est/1` and `storage::estimates()` stores
//! what it is given; this module is what joins them, so the progress screen
//! shows estimates the learner earned rather than only rows a test seeded. It
//! reads the profile's attempts, runs `estimate_profile` and appends one row per
//! skill. Nothing here decides a level, and no model output reaches it.
//!
//! History is append-only by design (see `crates/storage/src/estimates.rs`): a
//! recompute writes four rows and the current estimate of a skill is its latest
//! row. The rule itself filters the attempts (scored, authored, counting,
//! confident, recent), so nothing here hand-filters them.

use std::collections::BTreeMap;

use assessment_engine::{EstimateStatus, Level, Scorer, Skill};
use serde_json::json;
use storage::{Database, EstimateLevel, NewSkillEstimate, Timestamp};

use crate::core::AppCore;
use crate::data::ESTIMATE_SKILLS;
use crate::error::CoreResult;

/// How far back the attempts are read. The estimator applies the same 180-day
/// age rule again; this window only keeps the read small.
const WINDOW_DAYS: u32 = 180;

/// The four skills with their stored names, in the order `estimate_profile`
/// returns them. `ESTIMATE_SKILLS` is the one list of the names.
fn skills() -> impl Iterator<Item = (&'static str, Skill)> {
    ESTIMATE_SKILLS.iter().copied().zip(Skill::ALL)
}

/// The assessment crate's level for a stored attempt level.
fn engine_level(level: storage::Level) -> Level {
    match level {
        storage::Level::A1 => Level::A1,
        storage::Level::A2 => Level::A2,
        storage::Level::B1 => Level::B1,
        storage::Level::B2 => Level::B2,
        storage::Level::C1 => Level::C1,
        storage::Level::C2 => Level::C2,
    }
}

/// The stored level for an estimated one. `pre-A1` is how the progress screen
/// shows "working towards A1" (`apps/web/src/progress/model.ts`), so it is the
/// stored form of the engine's `WorkingTowardsA1`; storage has no status for it.
fn stored_level(level: Level) -> EstimateLevel {
    match level {
        Level::A1 => EstimateLevel::A1,
        Level::A2 => EstimateLevel::A2,
        Level::B1 => EstimateLevel::B1,
        Level::B2 => EstimateLevel::B2,
        Level::C1 => EstimateLevel::C1,
        Level::C2 => EstimateLevel::C2,
    }
}

/// One stored attempt as the estimator sees it.
///
/// A rubric-scored response carries its `response_id`, because its dimension
/// rows must count as one observation; a deterministic row has none and is its
/// own. A `NULL` score or confidence becomes 0.0, which the engine's
/// eligibility rule then refuses: the conservative direction.
fn engine_attempt(row: &storage::Attempt, skill: Skill) -> assessment_engine::Attempt {
    assessment_engine::Attempt {
        response_id: (row.scorer == storage::Scorer::RubricLlm).then(|| row.response_id.clone()),
        skill,
        level: engine_level(row.level),
        activity_id: row.activity_id.clone(),
        // A deleted session leaves its attempts behind with a `NULL` session id.
        // The string is only ever counted, never shown.
        session_id: row
            .session_id
            .map_or_else(|| "none".to_owned(), |id| id.to_string()),
        scorer: match row.scorer {
            storage::Scorer::Deterministic => Scorer::Deterministic,
            storage::Scorer::RubricLlm => Scorer::RubricLlm,
            storage::Scorer::PronEngine => Scorer::PronEngine,
        },
        normalized: row.normalized.unwrap_or(0.0),
        confidence: row.confidence.unwrap_or(0.0),
        status: match row.status {
            storage::AttemptStatus::Scored => assessment_engine::AttemptStatus::Scored,
            storage::AttemptStatus::PendingLlm => assessment_engine::AttemptStatus::PendingLlm,
            storage::AttemptStatus::NeedsReview => assessment_engine::AttemptStatus::NeedsReview,
            storage::AttemptStatus::Insufficient => assessment_engine::AttemptStatus::Insufficient,
            storage::AttemptStatus::Rejected => assessment_engine::AttemptStatus::Rejected,
        },
        origin: match row.origin {
            storage::AttemptOrigin::Authored => assessment_engine::Origin::Authored,
            storage::AttemptOrigin::Generated => assessment_engine::Origin::Generated,
            storage::AttemptOrigin::FreeMode => assessment_engine::Origin::FreeMode,
        },
        counts_toward_estimate: row.counts_toward_estimate,
        // The productive tasks of the assessment spec: a rubric-scored roleplay
        // counts as a productive response, an unscored one does not.
        productive_response: matches!(
            row.activity_type.as_str(),
            "guided_speaking" | "guided_writing" | "mediation"
        ) || (row.activity_type == "roleplay"
            && row.scorer == storage::Scorer::RubricLlm),
        created_at: row.created_at.unix_seconds(),
    }
}

/// The row to store for one estimated skill.
fn stored_row(
    profile_id: i64,
    name: &str,
    estimate: &assessment_engine::SkillEstimate,
    now: Timestamp,
) -> NewSkillEstimate {
    let (level, status, confidence) = match estimate.status {
        EstimateStatus::Estimated => (
            estimate.level.map(stored_level),
            storage::EstimateStatus::Estimated,
            Some(estimate.confidence),
        ),
        EstimateStatus::PlacementOnly => (
            estimate.level.map(stored_level),
            storage::EstimateStatus::PlacementOnly,
            Some(estimate.confidence),
        ),
        // "Working towards A1" is a level, not a status, on the progress screen:
        // the engine's status has no stored variant, and `pre-A1` renders as
        // exactly this phrase.
        EstimateStatus::WorkingTowardsA1 => (
            Some(EstimateLevel::PreA1),
            storage::EstimateStatus::Estimated,
            Some(estimate.confidence),
        ),
        EstimateStatus::InsufficientEvidence => {
            (None, storage::EstimateStatus::InsufficientEvidence, None)
        }
    };
    NewSkillEstimate {
        profile_id,
        skill: name.to_owned(),
        level,
        status,
        confidence,
        evidence_count: i64::try_from(estimate.scored_tasks).unwrap_or(i64::MAX),
        algorithm_version: estimate.algorithm.clone(),
        detail: Some(json!({
            "sessions": estimate.sessions,
            "band": estimate.band,
        })),
        computed_at: now,
    }
}

/// Recomputes the four skill estimates of `profile_id` and appends one row per
/// skill. The caller passes `now` so the window and the stored timestamp are one
/// instant.
pub async fn recompute_estimates(
    db: &Database,
    profile_id: i64,
    now: Timestamp,
) -> CoreResult<Vec<storage::SkillEstimate>> {
    let since = now
        .minus_days(WINDOW_DAYS)
        .map_err(storage::StorageError::from)?;
    let mut attempts = Vec::new();
    for (name, skill) in skills() {
        for row in db
            .attempts()
            .for_skill_since(profile_id, name, &since)
            .await?
        {
            attempts.push(engine_attempt(&row, skill));
        }
    }
    let profile =
        assessment_engine::estimate_profile(&attempts, &BTreeMap::new(), now.unix_seconds());
    let mut stored = Vec::with_capacity(profile.skills.len());
    // `profile.skills` is in `Skill::ALL` order, the order `skills()` holds too.
    for (estimate, (name, _)) in profile.skills.iter().zip(skills()) {
        stored.push(
            db.estimates()
                .insert(&stored_row(profile_id, name, estimate, now))
                .await?,
        );
    }
    Ok(stored)
}

impl AppCore {
    /// Recomputes the estimates of the learner's profile as of now. Called when
    /// a session ends and after a scoring backlog was drained.
    pub async fn recompute_estimates(&self) -> CoreResult<Vec<storage::SkillEstimate>> {
        recompute_estimates(&self.db, self.profile_id(), self.clock().now()).await
    }
}
