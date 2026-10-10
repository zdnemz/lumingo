#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! The estimate recompute (S5-05's production caller): what a session end and a
//! scoring backlog drain write to `skill_estimates`.

mod common;

use app_core::AppCore;
use app_core::api::{
    ActivityAnswer, SessionKind, SessionStatus, StopRequest, SubmitActivityRequest,
};
use common::seed::test_core_with_unit;
use common::sessions::{Setup, rig, unit_request};
use common::test_core;
use storage::{AttemptOrigin, AttemptStatus, Level, NewAttempt, NewSession, Scorer};

const UNIT: &str = "a1-u01";

fn submit(session_id: i64, activity_id: &str, answer: ActivityAnswer) -> SubmitActivityRequest {
    SubmitActivityRequest {
        session_id,
        activity_id: activity_id.to_owned(),
        answer,
    }
}

async fn seed_session(core: &AppCore) -> i64 {
    core.database()
        .sessions()
        .create(&NewSession {
            profile_id: core.profile_id(),
            kind: storage::SessionKind::Lesson,
            unit_id: Some(UNIT.to_owned()),
            activity_id: None,
            mode: None,
            provider_profile_id: None,
            app_version: "0.0.0-test".to_owned(),
            started_at: core.clock().now(),
        })
        .await
        .expect("session")
        .id
}

#[allow(clippy::too_many_arguments)]
async fn seed_attempt(
    core: &AppCore,
    session_id: i64,
    activity_id: &str,
    response_id: &str,
    skill: &str,
    normalized: f64,
    origin: AttemptOrigin,
    counts_toward_estimate: bool,
) {
    let at = core.clock().now();
    core.database()
        .attempts()
        .insert(&NewAttempt {
            profile_id: core.profile_id(),
            session_id: Some(session_id),
            unit_id: Some(UNIT.to_owned()),
            activity_id: activity_id.to_owned(),
            activity_type: "mcq".to_owned(),
            response_id: response_id.to_owned(),
            origin,
            level: Level::A1,
            skill: skill.to_owned(),
            dimension: "overall".to_owned(),
            scorer: Scorer::Deterministic,
            scorer_version: "mcq/1".to_owned(),
            raw_score: Some(normalized),
            max_score: Some(1.0),
            normalized: Some(normalized),
            confidence: Some(1.0),
            status: AttemptStatus::Scored,
            counts_toward_estimate,
            created_at: at,
        })
        .await
        .expect("attempt");
}

#[tokio::test]
async fn an_empty_history_writes_insufficient_evidence_for_all_four_skills() {
    let t = test_core().await;
    let stored = t.core.recompute_estimates().await.unwrap();
    assert_eq!(stored.len(), 4, "one row per skill");

    let latest = t
        .core
        .database()
        .estimates()
        .latest_per_skill(t.core.profile_id())
        .await
        .unwrap();
    let names: Vec<&str> = latest.iter().map(|row| row.skill.as_str()).collect();
    assert_eq!(names, ["listening", "reading", "speaking", "writing"]);
    for row in &latest {
        assert_eq!(
            row.status,
            storage::EstimateStatus::InsufficientEvidence,
            "{}",
            row.skill
        );
        assert_eq!(row.level, None, "{}", row.skill);
        assert_eq!(row.confidence, None, "{}", row.skill);
        assert_eq!(row.evidence_count, 0, "{}", row.skill);
        assert_eq!(row.algorithm_version, "est/1", "{}", row.skill);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_end_writes_an_estimate_row_per_skill() {
    // The unit's first listening item is played aloud, so this rig needs speech
    // output for it to be answerable at all; the listening answer is what makes
    // the evidence count observable.
    let r = rig(Setup {
        speech: Some(Vec::new()),
        ..Setup::default()
    })
    .await;
    let view = r.start(unit_request(SessionKind::Lesson, UNIT)).await;
    let done = r
        .core()
        .submit_activity(submit(
            view.id,
            "a01-listen-question",
            ActivityAnswer::Choice { index: 0 },
        ))
        .await
        .unwrap();
    assert!(done.result.is_some(), "the answer was scored");

    // The checkpoint still has activities to answer, so the run ends aborted:
    // its scored attempts are real rows all the same, and the end recomputes.
    let ended = r
        .core()
        .stop_session(view.id, StopRequest::default())
        .await
        .unwrap();
    assert_eq!(ended.status, SessionStatus::Aborted);

    let latest = r
        .core()
        .database()
        .estimates()
        .latest_per_skill(r.core().profile_id())
        .await
        .unwrap();
    assert_eq!(latest.len(), 4, "the session end wrote one row per skill");
    let listening = latest
        .iter()
        .find(|row| row.skill == "listening")
        .expect("listening");
    assert_eq!(
        listening.evidence_count, 1,
        "the one listening answer is the evidence"
    );
    assert_eq!(
        listening.status,
        storage::EstimateStatus::InsufficientEvidence,
        "one task cannot secure A1"
    );
    assert_eq!(listening.algorithm_version, "est/1");
}

#[tokio::test]
async fn only_authored_counting_attempts_reach_the_estimate() {
    let t = test_core_with_unit().await;
    let first = seed_session(&t.core).await;
    let second = seed_session(&t.core).await;
    // Eight counting attempts over two sessions and three activities: enough
    // weight, enough activities and enough sessions to secure A1.
    for n in 0..8 {
        seed_attempt(
            &t.core,
            if n % 2 == 0 { first } else { second },
            &format!("act-{}", n % 3),
            &format!("resp-{n}"),
            "listening",
            1.0,
            AttemptOrigin::Authored,
            true,
        )
        .await;
    }
    // A generated item and a free-mode answer never count, even with a score.
    for (activity, origin) in [
        ("gen-1", AttemptOrigin::Generated),
        ("chat-1", AttemptOrigin::FreeMode),
    ] {
        seed_attempt(
            &t.core,
            first,
            activity,
            &format!("resp-{activity}"),
            "listening",
            1.0,
            origin,
            false,
        )
        .await;
    }

    let stored = t.core.recompute_estimates().await.unwrap();
    let listening = stored
        .iter()
        .find(|row| row.skill == "listening")
        .expect("listening");
    assert_eq!(listening.status, storage::EstimateStatus::Estimated);
    assert_eq!(listening.level, Some(storage::EstimateLevel::A1));
    assert_eq!(
        listening.evidence_count, 8,
        "the generated and free-mode rows are not evidence"
    );

    // A second recompute appends: history is never upserted.
    t.core.recompute_estimates().await.unwrap();
    let history = t
        .core
        .database()
        .estimates()
        .history(t.core.profile_id(), "listening")
        .await
        .unwrap();
    assert_eq!(history.len(), 2, "one row per recompute");
    assert_eq!(history[1].level, Some(storage::EstimateLevel::A1));
}

#[tokio::test]
async fn working_towards_a1_is_stored_as_pre_a1() {
    let t = test_core_with_unit().await;
    let first = seed_session(&t.core).await;
    let second = seed_session(&t.core).await;
    // Eight counting attempts that mostly fail: the weight is there, the lower
    // bound is not, so A1 is not secured and the learner is working towards it.
    for n in 0..8 {
        seed_attempt(
            &t.core,
            if n % 2 == 0 { first } else { second },
            &format!("act-{}", n % 3),
            &format!("resp-{n}"),
            "listening",
            if n < 2 { 1.0 } else { 0.0 },
            AttemptOrigin::Authored,
            true,
        )
        .await;
    }

    let stored = t.core.recompute_estimates().await.unwrap();
    let listening = stored
        .iter()
        .find(|row| row.skill == "listening")
        .expect("listening");
    // The progress screen renders `pre-A1` as "working towards A1", so that is
    // the stored level; the engine's status has no stored variant.
    assert_eq!(listening.level, Some(storage::EstimateLevel::PreA1));
    assert_eq!(listening.status, storage::EstimateStatus::Estimated);
    assert_eq!(listening.confidence, Some(0.0));
    assert_eq!(listening.evidence_count, 8);
}
