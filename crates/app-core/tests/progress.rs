#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! The progress read models.

mod common;

use app_core::api::{
    EstimateLevel, EstimateStatus, EvidenceKind, ReviewItemKind, SessionKind, SessionStatus,
    UnitStatus,
};
use app_core::error::CoreError;
use common::seed::{seed_session_with_attempt, test_core_with_unit};
use common::test_core;
use storage::{NewReviewItem, NewSkillEstimate, ReviewKind, Timestamp};

#[tokio::test]
async fn a_new_learner_has_an_empty_overview() {
    let t = test_core().await;
    let overview = t.core.progress().await.unwrap();
    assert!(overview.units.is_empty());
    assert!(overview.objectives.is_empty());
    assert!(overview.errors.is_empty());
    assert!(overview.reviews_due.is_empty() && !overview.reviews_due_truncated);
    assert!(overview.estimates.is_empty());
    assert!(overview.recent_sessions.is_empty());
}

#[tokio::test]
async fn the_overview_shows_what_storage_holds_and_nothing_computed() {
    let t = test_core_with_unit().await;
    let (session, _) = seed_session_with_attempt(&t.core, storage::SessionKind::Lesson).await;
    let db = t.core.database();
    let profile = t.core.profile_id();
    let at = t.clock_now();
    db.error_stats()
        .add(profile, "grammar.articles", 3, &at)
        .await
        .unwrap();
    db.review_schedule()
        .put(&NewReviewItem {
            profile_id: profile,
            item_kind: ReviewKind::Vocab,
            item_ref: "a1-u01/v1".to_owned(),
            due_at: Timestamp::parse("2026-10-04T08:00:00.000Z").unwrap(),
            interval_days: 1.0,
            ease: 2.5,
            reps: 1,
            lapses: 0,
            last_reviewed_at: None,
        })
        .await
        .unwrap();
    db.review_schedule()
        .put(&NewReviewItem {
            profile_id: profile,
            item_kind: ReviewKind::Grammar,
            item_ref: "a1-u01/g1".to_owned(),
            due_at: Timestamp::parse("2026-10-09T08:00:00.000Z").unwrap(),
            interval_days: 4.0,
            ease: 2.5,
            reps: 2,
            lapses: 0,
            last_reviewed_at: None,
        })
        .await
        .unwrap();
    db.estimates()
        .insert(&NewSkillEstimate {
            profile_id: profile,
            skill: "listening".to_owned(),
            level: None,
            status: storage::EstimateStatus::InsufficientEvidence,
            confidence: None,
            evidence_count: 1,
            algorithm_version: "est/1".to_owned(),
            detail: None,
            computed_at: at,
        })
        .await
        .unwrap();

    let overview = t.core.progress().await.unwrap();
    assert_eq!(overview.units.len(), 1);
    assert_eq!(overview.units[0].unit_id, "a1-u01");
    assert_eq!(overview.units[0].status, UnitStatus::InProgress);
    assert_eq!(overview.units[0].best_checkpoint, Some(0.5));
    assert_eq!(overview.objectives[0].objective_id, "a1-u01/o1-greet");
    assert_eq!(overview.objectives[0].mastery, 0.75);
    assert_eq!(overview.errors[0].category, "grammar.articles");
    assert_eq!(overview.errors[0].count, 3);
    // Only the item due now is listed.
    assert_eq!(overview.reviews_due.len(), 1);
    assert_eq!(overview.reviews_due[0].item_kind, ReviewItemKind::Vocab);
    // "Insufficient evidence" is shown as such, with no level.
    assert_eq!(overview.estimates.len(), 1);
    assert_eq!(
        overview.estimates[0].status,
        EstimateStatus::InsufficientEvidence
    );
    assert_eq!(overview.estimates[0].level, None);
    assert_eq!(overview.recent_sessions.len(), 1);
    assert_eq!(overview.recent_sessions[0].id, session.id);
    assert_eq!(overview.recent_sessions[0].kind, SessionKind::Lesson);
    assert_eq!(overview.recent_sessions[0].status, SessionStatus::Active);
}

#[tokio::test]
async fn the_newest_estimate_of_a_skill_is_the_one_shown() {
    let t = test_core().await;
    let db = t.core.database();
    for (computed, level, status) in [
        (
            "2026-10-01T08:00:00.000Z",
            None,
            storage::EstimateStatus::InsufficientEvidence,
        ),
        (
            "2026-10-03T08:00:00.000Z",
            Some(storage::EstimateLevel::A2),
            storage::EstimateStatus::Estimated,
        ),
    ] {
        db.estimates()
            .insert(&NewSkillEstimate {
                profile_id: t.core.profile_id(),
                skill: "reading".to_owned(),
                level,
                status,
                confidence: level.map(|_| 0.8),
                evidence_count: 12,
                algorithm_version: "est/1".to_owned(),
                detail: None,
                computed_at: Timestamp::parse(computed).unwrap(),
            })
            .await
            .unwrap();
    }
    let overview = t.core.progress().await.unwrap();
    assert_eq!(overview.estimates.len(), 1);
    assert_eq!(overview.estimates[0].level, Some(EstimateLevel::A2));
    assert_eq!(overview.estimates[0].confidence, Some(0.8));
}

#[tokio::test]
async fn due_reviews_are_cut_at_fifty_and_say_so() {
    let t = test_core_with_unit().await;
    for n in 0..51 {
        t.core
            .database()
            .review_schedule()
            .put(&NewReviewItem {
                profile_id: t.core.profile_id(),
                item_kind: ReviewKind::Vocab,
                item_ref: format!("a1-u01/v{n}"),
                due_at: Timestamp::parse("2026-10-04T08:00:00.000Z").unwrap(),
                interval_days: 1.0,
                ease: 2.5,
                reps: 1,
                lapses: 0,
                last_reviewed_at: None,
            })
            .await
            .unwrap();
    }
    let overview = t.core.progress().await.unwrap();
    assert_eq!(overview.reviews_due.len(), 50);
    assert!(overview.reviews_due_truncated);
}

#[tokio::test]
async fn evidence_of_an_attempt_is_returned_and_unknown_attempts_are_not_found() {
    let t = test_core_with_unit().await;
    let (_, attempt) = seed_session_with_attempt(&t.core, storage::SessionKind::Lesson).await;
    let shown = t.core.attempt_evidence(attempt.id).await.unwrap();
    assert_eq!(shown.attempt_id, attempt.id);
    assert_eq!(shown.skill, "listening");
    assert_eq!(shown.dimension, "accuracy");
    assert_eq!(shown.evidence.len(), 1);
    assert_eq!(shown.evidence[0].kind, EvidenceKind::ResponseText);
    assert_eq!(
        shown.evidence[0].content.as_deref(),
        Some("I am from Jakarta")
    );

    let missing = t.core.attempt_evidence(9999).await.unwrap_err();
    assert!(matches!(missing, CoreError::NotFound { .. }), "{missing:?}");
}
