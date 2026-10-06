#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! Lessons, checkpoints and drills over the session manager: which activities are
//! offered, which are listed as unavailable and why, how an answer is scored and
//! how a run ends.

mod common;

use app_core::api::{
    ActivityAnswer, ActivityBody, ActivityOutcomeView, ErrorCode, Feature, FeedbackView,
    ServerEvent, SessionKind, SessionStatus, StopRequest, SubmitActivityRequest, TurnPhase,
};
use app_core::error::CoreError;
use app_core::voice::testing::{ReplyScript, Step};
use common::sessions::{Setup, rig, unit_request};

const UNIT: &str = "a1-u01";

fn submit(session_id: i64, activity_id: &str, answer: ActivityAnswer) -> SubmitActivityRequest {
    SubmitActivityRequest {
        session_id,
        activity_id: activity_id.to_owned(),
        answer,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_lesson_offers_what_can_be_done_here_and_lists_the_rest_with_the_reason() {
    let r = rig(Setup::default()).await;
    let view = r.start(unit_request(SessionKind::Lesson, UNIT)).await;
    assert_eq!(view.unit_id.as_deref(), Some(UNIT));
    assert_eq!(view.activity_id.as_deref(), Some("a01-listen-question"));

    let next = r.core().next_activity(view.id).await.unwrap();
    assert_eq!(next.total, 17);
    assert_eq!(next.answered, 0);
    let first = next.activity.expect("an activity to do");
    assert_eq!(first.id, "a02-greeting-by-time", "a01 is played aloud");
    assert!(matches!(first.body, ActivityBody::Mcq { .. }));
    let ids: Vec<&str> = next
        .unavailable
        .iter()
        .map(|u| u.activity_id.as_str())
        .collect();
    assert_eq!(
        ids,
        [
            "a01-listen-question",
            "a06-dictation-from",
            "a07-read-aloud-thanks",
            "a08-pairs-th",
            "a09-shadow-dialogue",
            "a10-speak-introduce",
            "a15-listen-set-putu"
        ]
    );
    let reason = |id: &str| {
        next.unavailable
            .iter()
            .find(|u| u.activity_id == id)
            .map(|u| u.reason.clone())
            .unwrap()
    };
    assert!(reason("a01-listen-question").contains("speech output"));
    assert!(reason("a07-read-aloud-thanks").contains("phoneme model"));
    assert!(reason("a10-speak-introduce").contains("recording"));

    // The presentation holds no answer.
    let shown = serde_json::to_string(&first).unwrap();
    assert!(!shown.contains("answer_index"), "{shown}");
    assert!(!shown.contains("explanation"), "{shown}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_answer_is_scored_stored_and_reported_and_the_next_activity_follows() {
    let r = rig(Setup::default()).await;
    let view = r.start(unit_request(SessionKind::Lesson, UNIT)).await;
    let mark = r.mark().await;

    let done = r
        .core()
        .submit_activity(submit(
            view.id,
            "a02-greeting-by-time",
            ActivityAnswer::Choice { index: 1 },
        ))
        .await
        .unwrap();
    let result = done.result.expect("a result");
    assert_eq!(result.activity_id, "a02-greeting-by-time");
    assert_eq!(result.score, Some(1.0));
    assert!(matches!(
        result.outcome,
        ActivityOutcomeView::Deterministic { .. }
    ));
    r.events
        .wait("the feedback of the activity", |events| {
            events.iter().skip(mark).any(|e| {
                matches!(
                    e,
                    ServerEvent::FeedbackReady {
                        feedback: FeedbackView::Activity { result }, ..
                    } if result.activity_id == "a02-greeting-by-time"
                )
            })
        })
        .await;

    let wrong = r
        .core()
        .submit_activity(submit(
            view.id,
            "a03-gap-am",
            ActivityAnswer::Gaps {
                answers: vec!["is".to_owned(), "from".to_owned()],
            },
        ))
        .await
        .unwrap()
        .result
        .unwrap();
    assert!(
        wrong.score.expect("a score") < 1.0,
        "a wrong gap is not full marks"
    );

    let next = r.core().next_activity(view.id).await.unwrap();
    assert_eq!(next.answered, 2);
    assert_eq!(next.activity.unwrap().id, "a04-reorder-name");
    assert_eq!(
        r.core()
            .snapshot()
            .active_session
            .unwrap()
            .activity_id
            .as_deref(),
        Some("a04-reorder-name")
    );

    // The attempt is stored with its session; a lesson attempt counts.
    let attempts = r
        .core()
        .database()
        .attempts()
        .for_session(view.id)
        .await
        .unwrap();
    assert_eq!(attempts.len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_activity_that_cannot_be_done_here_is_refused_with_its_reason() {
    let r = rig(Setup::default()).await;
    let view = r.start(unit_request(SessionKind::Lesson, UNIT)).await;
    let refused = r
        .core()
        .submit_activity(submit(
            view.id,
            "a01-listen-question",
            ActivityAnswer::Choice { index: 0 },
        ))
        .await
        .unwrap_err();
    let body = refused.body();
    assert_eq!(body.error, ErrorCode::NotAvailable);
    assert_eq!(body.feature, Some(Feature::Speech));
    assert!(body.message.contains("speech output"), "{}", body.message);

    let not_here = r
        .core()
        .submit_activity(submit(view.id, "no-such-activity", ActivityAnswer::Done))
        .await
        .unwrap_err();
    assert!(
        matches!(not_here, CoreError::InvalidInput(_)),
        "{not_here:?}"
    );

    let bad_shape = r
        .core()
        .submit_activity(submit(
            view.id,
            "a02-greeting-by-time",
            ActivityAnswer::Order { tokens: vec![] },
        ))
        .await
        .unwrap_err();
    assert!(
        matches!(bad_shape, CoreError::InvalidInput(_)),
        "an answer of the wrong kind is refused, not scored: {bad_shape:?}"
    );
    let stored = r
        .core()
        .database()
        .attempts()
        .for_session(view.id)
        .await
        .unwrap();
    assert!(stored.is_empty(), "nothing was stored for the refusals");
}

#[tokio::test(flavor = "multi_thread")]
async fn with_speech_output_the_activities_played_aloud_are_offered() {
    let r = rig(Setup {
        speech: Some(Vec::new()),
        ..Setup::default()
    })
    .await;
    let view = r.start(unit_request(SessionKind::Lesson, UNIT)).await;
    assert!(view.speaking, "the lesson can be heard");
    let next = r.core().next_activity(view.id).await.unwrap();
    assert_eq!(next.activity.unwrap().id, "a01-listen-question");
    let ids: Vec<&str> = next
        .unavailable
        .iter()
        .map(|u| u.activity_id.as_str())
        .collect();
    assert_eq!(
        ids,
        ["a07-read-aloud-thanks", "a10-speak-introduce"],
        "a scored pronunciation drill and a spoken answer still need what this server lacks"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_roleplay_is_a_text_conversation_inside_the_run() {
    let r = rig(Setup {
        steps: vec![
            Step::Reply(ReplyScript::new(&["Hi! I am Sam."])),
            Step::Reply(ReplyScript::new(&["Nice to meet you, Dewi."])),
        ],
        ..Setup::default()
    })
    .await;
    let view = r.start(unit_request(SessionKind::Lesson, UNIT)).await;

    // The learner's lines have nowhere to go before the roleplay starts.
    let early = r
        .core()
        .send_text(view.id, "Hello".to_owned())
        .await
        .unwrap_err();
    assert!(matches!(early, CoreError::Conflict(_)), "{early:?}");

    let started = r
        .core()
        .submit_activity(submit(
            view.id,
            "a11-roleplay-classmate",
            ActivityAnswer::RoleplayStart,
        ))
        .await
        .unwrap();
    assert!(started.result.is_none());
    r.events
        .wait("the tutor's opening line", |events| {
            events.iter().any(|e| {
                matches!(
                    e,
                    ServerEvent::TurnState {
                        state: TurnPhase::Waiting,
                        tutor_turn_seq: Some(_),
                        ..
                    }
                )
            })
        })
        .await;
    let accepted = r
        .core()
        .send_text(view.id, "Hello, I am Dewi.".to_owned())
        .await
        .unwrap();
    assert_eq!(accepted.turn, 2);
    r.events.wait_for("AnalysisReady", 0).await;

    let finished = r
        .core()
        .submit_activity(submit(
            view.id,
            "a11-roleplay-classmate",
            ActivityAnswer::RoleplayFinish,
        ))
        .await
        .unwrap();
    let result = finished
        .result
        .expect("a result at the end of the roleplay");
    assert_eq!(result.activity_id, "a11-roleplay-classmate");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_checkpoint_plays_its_own_activities_and_an_unfinished_one_ends_unpassed() {
    let r = rig(Setup::default()).await;
    let view = r.start(unit_request(SessionKind::Checkpoint, UNIT)).await;
    let next = r.core().next_activity(view.id).await.unwrap();
    assert_eq!(next.total, 7, "the activities the unit's checkpoint names");
    assert_eq!(next.activity.as_ref().unwrap().id, "a02-greeting-by-time");

    r.core()
        .submit_activity(submit(
            view.id,
            "a02-greeting-by-time",
            ActivityAnswer::Choice { index: 1 },
        ))
        .await
        .unwrap();
    let ended = r
        .core()
        .stop_session(view.id, StopRequest::default())
        .await
        .unwrap();
    assert_eq!(ended.status, SessionStatus::Aborted, "left unfinished");
    let Some(FeedbackView::Unit { summary }) = ended.feedback else {
        panic!("a unit summary");
    };
    assert!(!summary.passed);
    assert_eq!(summary.answered, 1);
    assert_eq!(summary.rows.len(), 7);
    let stored = r
        .core()
        .database()
        .sessions()
        .get(view.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.kind, storage::SessionKind::Checkpoint);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_drill_needs_what_scores_it_and_says_what_is_missing() {
    let without = rig(Setup::default()).await;
    let refused = without
        .core()
        .start_session(unit_request(SessionKind::Drill, UNIT))
        .await
        .unwrap_err();
    let body = refused.body();
    assert_eq!(body.error, ErrorCode::NotAvailable);
    assert!(
        body.message.contains("phoneme model") || body.message.contains("recording"),
        "{}",
        body.message
    );
    assert!(without.core().snapshot().active_session.is_none());

    let with = rig(Setup {
        speech: Some(Vec::new()),
        ..Setup::default()
    })
    .await;
    let view = with.start(unit_request(SessionKind::Drill, UNIT)).await;
    assert_eq!(view.kind, SessionKind::Drill);
    let next = with.core().next_activity(view.id).await.unwrap();
    assert_eq!(next.total, 2, "the read-aloud and the shadowing");
    assert_eq!(next.activity.unwrap().id, "a09-shadow-dialogue");
    assert_eq!(next.unavailable.len(), 1);
    assert_eq!(next.unavailable[0].activity_id, "a07-read-aloud-thanks");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_unit_run_needs_a_unit_that_exists() {
    let r = rig(Setup::default()).await;
    let none = r
        .core()
        .start_session(app_core::api::StartSessionRequest {
            unit_id: None,
            ..unit_request(SessionKind::Lesson, UNIT)
        })
        .await
        .unwrap_err();
    assert!(matches!(none, CoreError::InvalidInput(_)), "{none:?}");
    let unknown = r
        .core()
        .start_session(unit_request(SessionKind::Lesson, "z9-u99"))
        .await
        .unwrap_err();
    assert!(matches!(unknown, CoreError::NotFound { .. }), "{unknown:?}");
    assert!(r.core().snapshot().active_session.is_none());
}
