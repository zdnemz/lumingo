#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! The progress read models.

mod common;

use app_core::api::{
    ActivityAnswer, AttemptStatusView, EstimateLevel, EstimateStatus, EvidenceKind, ReviewItemKind,
    SessionKind, SessionStatus, StopRequest, SubmitActivityRequest, UnitStatus,
};
use app_core::error::CoreError;
use common::seed::{seed_session_with_attempt, test_core_with_unit};
use common::sessions::{Setup, rig, unit_request};
use common::{TestCore, test_core};
use storage::{
    AttemptOrigin, AttemptStatus, Level, NewAttempt, NewReviewItem, NewSkillEstimate, ReviewKind,
    Scorer, Timestamp,
};

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
    assert!(
        shown.evidence[0]
            .content
            .as_deref()
            .is_some_and(|text| text.starts_with("I am from Jakarta"))
    );

    let missing = t.core.attempt_evidence(9999).await.unwrap_err();
    assert!(matches!(missing, CoreError::NotFound { .. }), "{missing:?}");
}

/// Inserts one listening attempt made at `created_at`.
async fn insert_attempt(
    t: &TestCore,
    response_id: &str,
    created_at: &str,
    status: AttemptStatus,
    counts: bool,
) -> i64 {
    t.core
        .database()
        .attempts()
        .insert(&NewAttempt {
            profile_id: t.core.profile_id(),
            session_id: None,
            unit_id: Some("a1-u01".to_owned()),
            activity_id: "act-1".to_owned(),
            activity_type: "mcq".to_owned(),
            response_id: response_id.to_owned(),
            origin: AttemptOrigin::Authored,
            level: Level::A1,
            skill: "listening".to_owned(),
            dimension: "accuracy".to_owned(),
            scorer: Scorer::Deterministic,
            scorer_version: "det/1".to_owned(),
            raw_score: Some(1.0),
            max_score: Some(1.0),
            normalized: Some(1.0),
            confidence: Some(1.0),
            status,
            counts_toward_estimate: counts,
            created_at: Timestamp::parse(created_at).unwrap(),
        })
        .await
        .unwrap()
        .id
}

#[tokio::test]
async fn counted_attempts_of_one_skill_are_newest_first_with_their_status() {
    let t = test_core_with_unit().await;
    insert_attempt(
        &t,
        "r-old",
        "2026-10-01T08:00:00.000Z",
        AttemptStatus::Scored,
        true,
    )
    .await;
    insert_attempt(
        &t,
        "r-new",
        "2026-10-03T08:00:00.000Z",
        AttemptStatus::PendingLlm,
        true,
    )
    .await;
    // A rejected score does not count, so it is not in the list.
    insert_attempt(
        &t,
        "r-rejected",
        "2026-10-04T08:00:00.000Z",
        AttemptStatus::Rejected,
        false,
    )
    .await;

    let list = t.core.skill_attempts("listening").await.unwrap();
    assert_eq!(list.skill, "listening");
    assert!(!list.truncated);
    assert_eq!(list.attempts.len(), 2);
    let newest = &list.attempts[0];
    assert_eq!(newest.activity_id, "act-1");
    assert_eq!(newest.activity_type, "mcq");
    assert_eq!(newest.skill, "listening");
    assert_eq!(newest.dimension, "accuracy");
    assert_eq!(newest.normalized, Some(1.0));
    assert_eq!(newest.status, AttemptStatusView::PendingLlm);
    assert_eq!(newest.created_at, "2026-10-03T08:00:00.000Z");
    assert_eq!(list.attempts[1].status, AttemptStatusView::Scored);

    // A valid skill with no attempts is an empty list; an unknown skill is refused.
    let empty = t.core.skill_attempts("reading").await.unwrap();
    assert!(empty.attempts.is_empty() && !empty.truncated);
    for bad in ["grammar", ""] {
        let refused = t.core.skill_attempts(bad).await.unwrap_err();
        assert!(
            matches!(refused, CoreError::InvalidInput(_)),
            "{bad:?}: {refused:?}"
        );
    }
}

#[tokio::test]
async fn counted_attempts_are_cut_at_fifty_and_say_so() {
    let t = test_core_with_unit().await;
    for n in 0..51 {
        insert_attempt(
            &t,
            &format!("r-{n}"),
            &format!("2026-10-01T08:{n:02}:00.000Z"),
            AttemptStatus::Scored,
            true,
        )
        .await;
    }
    let list = t.core.skill_attempts("listening").await.unwrap();
    assert_eq!(list.attempts.len(), 50);
    assert!(list.truncated);
    assert_eq!(list.attempts[0].created_at, "2026-10-01T08:50:00.000Z");
    assert_eq!(list.attempts[49].created_at, "2026-10-01T08:01:00.000Z");
}

/// Drops `a10-speak-introduce` from the checkpoint of the temp copy of the
/// example unit: offline there is no way to record a spoken answer, and the
/// checkpoint cannot be finished with an item left unanswered. The six review
/// items of the unit are untouched.
fn patch_checkpoint_speaking(r: &common::sessions::SessionRig) {
    let path = r.t.dir.path().join("units/a1-u01.json");
    let mut unit: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let ids = unit["checkpoint"]["activity_ids"].as_array_mut().unwrap();
    ids.retain(|id| id.as_str() != Some("a10-speak-introduce"));
    std::fs::write(&path, serde_json::to_string(&unit).unwrap()).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_completed_unit_run_enrols_its_review_items_and_they_are_due() {
    let r = rig(Setup {
        speech: Some(Vec::new()),
        ..Setup::default()
    })
    .await;
    patch_checkpoint_speaking(&r);
    r.core().reindex_curriculum().await.unwrap();
    let view = r.start(unit_request(SessionKind::Lesson, "a1-u01")).await;

    // The offered activities the checkpoint consists of, answered through the
    // public API. The two productive answers wait for a provider: the scripted
    // model answers nothing, so they are stored as pending.
    let answers: Vec<(&str, ActivityAnswer)> = vec![
        (
            "a02-greeting-by-time",
            ActivityAnswer::Choice { index: 1 },
        ),
        (
            "a03-gap-am",
            ActivityAnswer::Gaps {
                answers: vec!["am".to_owned(), "from".to_owned()],
            },
        ),
        (
            "a15-listen-set-putu",
            ActivityAnswer::Picks {
                picks: vec![Some(0), Some(1), Some(1)],
            },
        ),
        (
            "a14-read-set-class-chat",
            ActivityAnswer::Picks {
                picks: vec![Some(1), Some(2), Some(2), Some(0)],
            },
        ),
        (
            "a12-write-introduce",
            ActivityAnswer::Text {
                text: "Hello! My name is Dewi. I am from Bandung. Nice to meet you.".to_owned(),
            },
        ),
        (
            "a16-fix-missing-am",
            ActivityAnswer::Text {
                text: "I am Dewi.".to_owned(),
            },
        ),
    ];
    for (activity_id, answer) in answers {
        r.core()
            .submit_activity(SubmitActivityRequest {
                session_id: view.id,
                activity_id: activity_id.to_owned(),
                answer,
            })
            .await
            .unwrap();
    }
    let ended = r
        .core()
        .stop_session(view.id, StopRequest::default())
        .await
        .unwrap();
    assert_eq!(ended.status, SessionStatus::Completed);

    let overview = r.core().progress().await.unwrap();
    let refs: Vec<&str> = overview
        .reviews_due
        .iter()
        .map(|item| item.item_ref.as_str())
        .collect();
    assert_eq!(
        refs,
        [
            "a1-u01/v-good-evening",
            "a1-u01/v-nice-to-meet-you",
            "a1-u01/v-from",
            "a1-u01/v-see-you",
            "a1-u01/g-be-i-am",
            "a1-u01/p-th-voiceless",
        ]
    );
    let kinds: Vec<ReviewItemKind> = overview
        .reviews_due
        .iter()
        .map(|item| item.item_kind)
        .collect();
    assert_eq!(
        kinds,
        [
            ReviewItemKind::Vocab,
            ReviewItemKind::Vocab,
            ReviewItemKind::Vocab,
            ReviewItemKind::Vocab,
            ReviewItemKind::Grammar,
            ReviewItemKind::Pron,
        ]
    );
    let now = r.t.clock_now().to_string();
    assert!(overview.reviews_due.iter().all(|item| item.due_at == now));
    assert!(!overview.reviews_due_truncated);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unfinished_unit_run_leaves_the_review_queue_empty() {
    let r = rig(Setup::default()).await;
    let view = r
        .start(unit_request(SessionKind::Checkpoint, "a1-u01"))
        .await;
    r.core()
        .submit_activity(SubmitActivityRequest {
            session_id: view.id,
            activity_id: "a02-greeting-by-time".to_owned(),
            answer: ActivityAnswer::Choice { index: 1 },
        })
        .await
        .unwrap();
    let ended = r
        .core()
        .stop_session(view.id, StopRequest::default())
        .await
        .unwrap();
    assert_eq!(ended.status, SessionStatus::Aborted);
    let overview = r.core().progress().await.unwrap();
    assert!(overview.reviews_due.is_empty());
    assert!(!overview.reviews_due_truncated);
}
