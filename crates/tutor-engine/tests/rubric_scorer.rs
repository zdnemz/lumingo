#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! The rubric scorer service: one or two runs, the cross-checks, the confidence
//! table, review, the queue and the backlog. The model is a scripted fake.

mod common;

use std::sync::Arc;

use assessment_engine::{Level, Origin};
use common::{FakeLlm, make_profile, temp_db, test_clock};
use curriculum::validate::GrammarCheck;
use llm_client::{LlmError, TimeoutKind};
use serde_json::{Value, json};
use storage::{AttemptStatus, Database, EvidenceKind, LlmCallType, LlmOutcome};
use tokio_util::sync::CancellationToken;
use tutor_engine::{
    Dimension, EngineError, EvidenceRecorder, MAX_OUTPUT_TRIES, RubricDimension, RubricInputMode,
    RubricScorer, Runs, ScoreRequest, ScoreResult, ScorerEnv, Subject, WorkshopRubric,
    WorkshopTask,
};

const RESPONSE: &str = "Hello. My name is Dewi. I am from Bandung.";

/// Flags two fixed things, the second as a spelling finding.
struct FakeGrammar;

impl GrammarCheck for FakeGrammar {
    fn findings(&self, text: &str) -> Vec<String> {
        let mut out = Vec::new();
        for _ in text.matches("Bandung") {
            out.push("Grammar: a made-up finding".to_owned());
            out.push("Grammar: another made-up finding".to_owned());
            out.push("Grammar: a third made-up finding".to_owned());
        }
        if text.contains("Dewi") {
            out.push("Spelling: check 'Dewi'".to_owned());
        }
        out
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    db: Database,
    llm: Arc<FakeLlm>,
    scorer: RubricScorer,
    profile_id: i64,
}

fn env(db: &Database, llm: &Arc<FakeLlm>, grammar: bool, qualified: bool) -> ScorerEnv {
    ScorerEnv {
        client: llm.clone(),
        db: db.clone(),
        clock: test_clock(),
        model: "test-model".into(),
        provider_profile_id: None,
        grammar: grammar.then(|| Arc::new(FakeGrammar) as Arc<dyn GrammarCheck + Send + Sync>),
        word_levels: None,
        provider_qualified: qualified,
    }
}

async fn fixture_with(grammar: bool, qualified: bool) -> Fixture {
    let (dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let llm = FakeLlm::new();
    let scorer = RubricScorer::new(env(&db, &llm, grammar, qualified));
    Fixture {
        _dir: dir,
        db,
        llm,
        scorer,
        profile_id: profile.id,
    }
}

async fn fixture() -> Fixture {
    fixture_with(false, true).await
}

fn rubric() -> WorkshopRubric {
    WorkshopRubric {
        id: "rubric-a1-spoken-production".into(),
        version: 1,
        dimensions: [
            Dimension::TaskAchievement,
            Dimension::Range,
            Dimension::Accuracy,
            Dimension::Coherence,
        ]
        .into_iter()
        .map(|dimension| RubricDimension {
            dimension,
            bands: ["none", "little", "some", "enough", "more"].map(String::from),
        })
        .collect(),
    }
}

fn request(f: &Fixture, origin: Origin, runs: Runs) -> ScoreRequest {
    ScoreRequest {
        subject: Subject::new(
            f.profile_id,
            None,
            Some("a1-u01".into()),
            "a10-speak-introduce",
            "guided_speaking",
            Level::A1,
            "speaking",
            origin,
            f.scorer.recorder().new_response_id("a10"),
        ),
        rubric: rubric(),
        task: WorkshopTask {
            prompt: "Introduce yourself.".into(),
            content_points: vec![
                "a greeting".into(),
                "the speaker's name".into(),
                "where the speaker is from".into(),
            ],
            min_words: Some(5),
        },
        response: RESPONSE.to_owned(),
        input_mode: RubricInputMode::Text,
        runs,
        first_language: "Indonesian".into(),
        timing: None,
    }
}

fn reply(bands: [&str; 4]) -> Value {
    let dims = ["task_achievement", "range", "accuracy", "coherence"];
    json!({
        "dimension_scores": dims.iter().zip(bands).map(|(d, b)| json!({
            "dimension": d, "band": b,
            "evidence_quotes": ["My name is Dewi"], "reason": "A reason."
        })).collect::<Vec<_>>(),
        "content_points": [
            { "point": "a greeting", "covered": true, "quote": "Hello" },
            { "point": "the speaker's name", "covered": true, "quote": "My name is Dewi" },
            { "point": "where the speaker is from", "covered": true, "quote": "I am from Bandung" }
        ],
        "on_task": true,
        "feedback_en": "You said your name clearly. Add where you live.",
        "feedback_l1": "Kamu menyebut namamu dengan jelas. Tambahkan tempat tinggalmu."
    })
}

fn cancel() -> CancellationToken {
    CancellationToken::new()
}

async fn attempts(f: &Fixture, request: &ScoreRequest) -> Vec<storage::Attempt> {
    f.db.attempts()
        .by_response(&request.subject.response_id)
        .await
        .unwrap()
}

#[tokio::test]
async fn one_run_scores_each_dimension_and_stores_authored_attempts_that_count() {
    let f = fixture().await;
    let request = request(&f, Origin::Authored, Runs::One);
    f.llm.queue_structured(Ok(reply(["3", "2", "3", "4"])));
    let result = f.scorer.score(request.clone(), &cancel()).await.unwrap();
    let ScoreResult::Scored { recorded, outcome } = result else {
        panic!("scored");
    };
    assert_eq!(f.llm.structured_calls(), 1);
    assert_eq!(outcome.runs, 1);
    assert!((outcome.confidence - 0.8).abs() < 1e-9);
    assert_eq!(recorded.attempts.len(), 4);
    let range = recorded
        .attempts
        .iter()
        .find(|a| a.dimension == "range")
        .unwrap();
    assert_eq!(range.normalized, Some(0.5));
    assert_eq!(range.status, AttemptStatus::Scored);
    assert!(range.counts_toward_estimate);
    assert!(range.scorer_version.ends_with("+test-model"));
    // The call is logged without any text.
    let calls = f.db.diagnostics().recent_llm_calls(10).await.unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].call_type, LlmCallType::RubricScore);
    assert_eq!(calls[0].outcome, LlmOutcome::Ok);
    // The request carried the rubric and the response and the text mode.
    let seen = f.llm.structured_requests();
    let user = &seen[0].messages[0].content;
    assert!(user.contains(r#""input_mode":"text""#));
    assert!(user.contains("My name is Dewi"));
    assert!(user.contains("Introduce yourself."));
}

#[tokio::test]
async fn generated_and_free_mode_responses_are_scored_and_never_count() {
    for origin in [Origin::Generated, Origin::FreeMode] {
        let f = fixture().await;
        let request = request(&f, origin, Runs::One);
        f.llm.queue_structured(Ok(reply(["3", "3", "3", "3"])));
        let ScoreResult::Scored { recorded, .. } =
            f.scorer.score(request, &cancel()).await.unwrap()
        else {
            panic!("scored");
        };
        assert!(
            recorded
                .attempts
                .iter()
                .all(|a| a.status == AttemptStatus::Scored && !a.counts_toward_estimate),
            "{origin:?}"
        );
    }
}

#[tokio::test]
async fn two_runs_that_agree_exactly_raise_the_confidence_by_a_tenth() {
    let f = fixture().await;
    let request = request(&f, Origin::Authored, Runs::Two);
    f.llm.queue_structured(Ok(reply(["3", "3", "3", "3"])));
    f.llm.queue_structured(Ok(reply(["3", "3", "3", "3"])));
    let ScoreResult::Scored { recorded, outcome } =
        f.scorer.score(request, &cancel()).await.unwrap()
    else {
        panic!("scored");
    };
    assert_eq!(f.llm.structured_calls(), 2);
    assert!(outcome.runs_agreed);
    assert!((outcome.confidence - 0.9).abs() < 1e-9);
    assert!(
        recorded
            .attempts
            .iter()
            .all(|a| (a.confidence.unwrap() - 0.9).abs() < 1e-9)
    );
    let first =
        f.db.evidence()
            .for_attempt(recorded.attempts[0].id)
            .await
            .unwrap();
    let metric = first
        .iter()
        .find(|e| e.kind == EvidenceKind::Metric)
        .and_then(|e| e.data.clone())
        .unwrap();
    assert_eq!(metric["details"]["runs"], 2);
    assert_eq!(metric["details"]["runs_agreed"], true);
}

#[tokio::test]
async fn two_runs_one_band_apart_use_the_mean_and_two_apart_need_review() {
    let f = fixture().await;
    let request = request(&f, Origin::Authored, Runs::Two);
    f.llm.queue_structured(Ok(reply(["3", "3", "3", "1"])));
    f.llm.queue_structured(Ok(reply(["4", "3", "3", "3"])));
    let ScoreResult::Scored { recorded, outcome } =
        f.scorer.score(request, &cancel()).await.unwrap()
    else {
        panic!("scored");
    };
    assert!(!outcome.is_complete());
    let by = |dimension: &str| {
        recorded
            .attempts
            .iter()
            .find(|a| a.dimension == dimension)
            .unwrap()
    };
    assert_eq!(by("task_achievement").raw_score, Some(3.5));
    assert_eq!(by("task_achievement").normalized, Some(0.875));
    assert_eq!(by("coherence").status, AttemptStatus::NeedsReview);
    assert_eq!(by("coherence").normalized, None);
    assert!(!by("coherence").counts_toward_estimate);
    // The disagreement is recorded as evidence on that dimension.
    let rows =
        f.db.evidence()
            .for_attempt(by("coherence").id)
            .await
            .unwrap();
    assert!(rows.iter().any(|e| {
        e.data
            .as_ref()
            .is_some_and(|d| d["run_bands"] == json!([1, 3]))
    }));
}

#[tokio::test]
async fn x1_a_band_without_a_valid_quote_gets_one_rerun() {
    let f = fixture().await;
    let request = request(&f, Origin::Authored, Runs::One);
    let mut bad = reply(["3", "3", "3", "3"]);
    bad["dimension_scores"][1]["evidence_quotes"] = json!(["words the learner never said"]);
    f.llm.queue_structured(Ok(bad));
    f.llm.queue_structured(Ok(reply(["3", "3", "3", "3"])));
    let ScoreResult::Scored { recorded, outcome } =
        f.scorer.score(request, &cancel()).await.unwrap()
    else {
        panic!("scored");
    };
    assert_eq!(f.llm.structured_calls(), 2, "one call and one rerun");
    assert!(outcome.is_complete());
    assert!(
        recorded
            .attempts
            .iter()
            .all(|a| a.status == AttemptStatus::Scored)
    );

    // A dimension that fails twice is left for review.
    let f = fixture().await;
    let request = request_for(&f);
    let mut bad = reply(["3", "3", "3", "3"]);
    bad["dimension_scores"][1]["evidence_quotes"] = json!(["invented"]);
    f.llm.queue_structured(Ok(bad.clone()));
    f.llm.queue_structured(Ok(bad));
    let ScoreResult::Scored { recorded, .. } = f.scorer.score(request, &cancel()).await.unwrap()
    else {
        panic!("scored");
    };
    let range = recorded
        .attempts
        .iter()
        .find(|a| a.dimension == "range")
        .unwrap();
    assert_eq!(range.status, AttemptStatus::NeedsReview);
}

fn request_for(f: &Fixture) -> ScoreRequest {
    request(f, Origin::Authored, Runs::One)
}

/// Eleven words, so the finding rate of X5 means something (it needs ten).
fn longer(f: &Fixture) -> ScoreRequest {
    let mut long = request_for(f);
    long.response = "Hello. My name is Dewi. I am from Bandung in Indonesia today.".into();
    long
}

#[tokio::test]
async fn x2_x3_and_x7_change_the_score() {
    // X2: below the minimum word count caps task achievement at 2.
    let f = fixture().await;
    let mut short = request_for(&f);
    short.task.min_words = Some(50);
    f.llm.queue_structured(Ok(reply(["4", "3", "3", "3"])));
    let ScoreResult::Scored { recorded, .. } = f.scorer.score(short, &cancel()).await.unwrap()
    else {
        panic!("scored");
    };
    let ta = recorded
        .attempts
        .iter()
        .find(|a| a.dimension == "task_achievement")
        .unwrap();
    assert_eq!(ta.raw_score, Some(2.0));

    // X7: off task makes every band 0 and asks for another try.
    let f = fixture().await;
    let mut off = reply(["3", "3", "3", "3"]);
    off["on_task"] = json!(false);
    f.llm.queue_structured(Ok(off));
    let ScoreResult::Scored { recorded, outcome } =
        f.scorer.score(request_for(&f), &cancel()).await.unwrap()
    else {
        panic!("scored");
    };
    assert!(outcome.try_again());
    assert!(recorded.attempts.iter().all(|a| a.normalized == Some(0.0)));
}

#[tokio::test]
async fn x4_and_x5_only_lower_the_confidence_and_x5_needs_a_checker() {
    let mut rich = reply(["3", "3", "4", "3"]);
    rich["dimension_scores"][2]["evidence_quotes"] = json!(["I am from Bandung"]);
    // With a checker: three grammar findings in nine words, and an accuracy of 4.
    let f = fixture_with(true, true).await;
    f.llm.queue_structured(Ok(rich.clone()));
    let ScoreResult::Scored { outcome, recorded } =
        f.scorer.score(longer(&f), &cancel()).await.unwrap()
    else {
        panic!("scored");
    };
    assert_eq!(outcome.alarms, [tutor_engine::Alarm::Accuracy]);
    assert!((outcome.confidence - 0.6).abs() < 1e-9);
    let accuracy = recorded
        .attempts
        .iter()
        .find(|a| a.dimension == "accuracy")
        .unwrap();
    assert_eq!(accuracy.raw_score, Some(4.0), "the band is not changed");

    // Without one the check is skipped.
    let f = fixture_with(false, true).await;
    f.llm.queue_structured(Ok(rich));
    let ScoreResult::Scored { outcome, .. } = f.scorer.score(longer(&f), &cancel()).await.unwrap()
    else {
        panic!("scored");
    };
    assert!(outcome.alarms.is_empty());
    assert!((outcome.confidence - 0.8).abs() < 1e-9);
}

#[tokio::test]
async fn spelling_findings_are_ignored_for_a_voice_response() {
    // Eight words, so the finding rate counts: 3 grammar findings = 37 per 100 words.
    // The spelling finding must not change the count for voice.
    let f = fixture_with(true, true).await;
    let mut voice = request_for(&f);
    voice.input_mode = RubricInputMode::Voice;
    voice.response = "hello my name is dewi i am from Bandung".into();
    f.llm.queue_structured(Ok(reply(["3", "3", "3", "3"])));
    let ScoreResult::Scored { recorded, .. } = f.scorer.score(voice, &cancel()).await.unwrap()
    else {
        panic!("scored");
    };
    let rows =
        f.db.evidence()
            .for_attempt(recorded.attempts[0].id)
            .await
            .unwrap();
    let metric = rows
        .iter()
        .find(|e| e.kind == EvidenceKind::Metric)
        .and_then(|e| e.data.clone())
        .unwrap();
    assert_eq!(metric["details"]["input_mode"], "voice");
    assert_eq!(metric["details"]["metrics"]["grammar_findings"], 3);
    let seen = f.llm.structured_requests();
    assert!(
        seen[0].messages[0]
            .content
            .contains(r#""input_mode":"voice""#)
    );
}

#[tokio::test]
async fn a_model_cannot_state_a_level_in_the_feedback_it_returns() {
    let f = fixture().await;
    let mut cheeky = reply(["3", "3", "3", "3"]);
    cheeky["feedback_en"] = json!("Your English is B2 and 85 percent correct.");
    cheeky["feedback_l1"] = json!("Levelmu adalah C1.");
    f.llm.queue_structured(Ok(cheeky));
    let ScoreResult::Scored { outcome, .. } = f.scorer.score(longer(&f), &cancel()).await.unwrap()
    else {
        panic!("scored");
    };
    assert_eq!(outcome.feedback_en, "");
    assert_eq!(outcome.feedback_l1, "");
}

#[tokio::test]
async fn an_unreachable_provider_queues_the_response_as_pending() {
    let f = fixture().await;
    let request = request_for(&f);
    f.llm
        .queue_structured(Err(LlmError::Timeout(TimeoutKind::Total)));
    let result = f.scorer.score(request.clone(), &cancel()).await.unwrap();
    let ScoreResult::Queued { recorded } = result else {
        panic!("queued");
    };
    assert_eq!(recorded.attempts.len(), 4);
    assert!(
        recorded
            .attempts
            .iter()
            .all(|a| a.status == AttemptStatus::PendingLlm && a.normalized.is_none())
    );
    let queue = f.db.pending_scoring().oldest(10).await.unwrap();
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0].payload["kind"], "rubric_response");
    assert_eq!(queue[0].payload["request"]["response"], RESPONSE);
    assert_eq!(
        queue[0].payload["request"]["subject"]["response_id"],
        request.subject.response_id
    );
}

#[tokio::test]
async fn a_cancelled_call_queues_the_response_and_returns_the_cancellation() {
    let f = fixture().await;
    let request = request_for(&f);
    f.llm.queue_structured(Err(LlmError::Cancelled));
    let error = f
        .scorer
        .score(request.clone(), &cancel())
        .await
        .unwrap_err();
    assert!(matches!(error, EngineError::Llm(LlmError::Cancelled)));
    assert_eq!(attempts(&f, &request).await.len(), 4);
    assert_eq!(f.db.pending_scoring().oldest(10).await.unwrap().len(), 1);
}

#[tokio::test]
async fn an_empty_response_is_refused_before_anything_is_sent_or_stored() {
    let f = fixture().await;
    let mut empty = request_for(&f);
    empty.response = "  \n ".into();
    let error = f.scorer.score(empty.clone(), &cancel()).await.unwrap_err();
    assert!(matches!(error, EngineError::Activity(_)));
    assert_eq!(f.llm.structured_calls(), 0);
    assert!(attempts(&f, &empty).await.is_empty());
}

#[tokio::test]
async fn the_backlog_is_scored_when_the_provider_is_back_and_the_flag_follows_the_confidence() {
    let f = fixture().await;
    let first = request_for(&f);
    let second = request_for(&f);
    for request in [&first, &second] {
        f.llm
            .queue_structured(Err(LlmError::Timeout(TimeoutKind::Total)));
        f.scorer.score(request.clone(), &cancel()).await.unwrap();
    }
    assert_eq!(f.db.pending_scoring().oldest(10).await.unwrap().len(), 2);
    let pending = attempts(&f, &first).await;
    assert!(pending.iter().all(|a| a.counts_toward_estimate));

    f.llm.queue_structured(Ok(reply(["3", "3", "3", "3"])));
    f.llm.queue_structured(Ok(reply(["2", "2", "2", "2"])));
    let report = f.scorer.score_pending_backlog(&cancel()).await.unwrap();
    assert_eq!(report.scored, 2);
    assert_eq!(report.remaining, 0);
    assert!(!report.provider_unreachable);
    assert!(f.db.pending_scoring().oldest(10).await.unwrap().is_empty());

    let done = attempts(&f, &first).await;
    assert_eq!(done.len(), 4);
    assert!(done.iter().all(|a| a.status == AttemptStatus::Scored));
    assert!(done.iter().all(|a| a.counts_toward_estimate));
    assert!(done[0].scorer_version.ends_with("+test-model"));
    assert_eq!(done[0].normalized, Some(0.75));
    let other = attempts(&f, &second).await;
    assert_eq!(other[0].normalized, Some(0.5));
    // Each response keeps its own text as evidence, written when it was queued.
    let rows = f.db.evidence().for_attempt(done[0].id).await.unwrap();
    assert_eq!(rows[0].kind, EvidenceKind::ResponseText);
    assert_eq!(rows[0].content.as_deref(), Some(RESPONSE));
}

#[tokio::test]
async fn a_late_score_exactly_at_the_confidence_floor_still_counts() {
    // The flag turning off below the floor is tested on the recorder, because
    // only a repaired reply can get that low and the fake client never repairs.
    let f = fixture_with(true, false).await;
    let request = longer(&f);
    f.llm
        .queue_structured(Err(LlmError::Timeout(TimeoutKind::Total)));
    f.scorer.score(request.clone(), &cancel()).await.unwrap();
    let mut rich = reply(["3", "3", "4", "3"]);
    rich["dimension_scores"][2]["evidence_quotes"] = json!(["I am from Bandung"]);
    f.llm.queue_structured(Ok(rich));
    f.scorer.score_pending_backlog(&cancel()).await.unwrap();
    let rows = attempts(&f, &request).await;
    // 0.5 (not qualified) - 0.2 (alarm) = 0.3, which is the floor and still counts.
    assert!(
        rows.iter()
            .all(|a| (a.confidence.unwrap() - 0.3).abs() < 1e-9)
    );
    assert!(rows.iter().all(|a| a.counts_toward_estimate));
}

#[tokio::test]
async fn a_provider_that_is_still_away_is_asked_once_and_the_queue_is_left_alone() {
    let f = fixture().await;
    for _ in 0..3 {
        let request = request_for(&f);
        f.llm
            .queue_structured(Err(LlmError::Timeout(TimeoutKind::Total)));
        f.scorer.score(request, &cancel()).await.unwrap();
    }
    assert_eq!(f.llm.structured_calls(), 3);
    f.llm
        .queue_structured(Err(LlmError::Timeout(TimeoutKind::Total)));
    let report = f.scorer.score_pending_backlog(&cancel()).await.unwrap();
    assert!(report.provider_unreachable);
    assert_eq!(report.scored, 0);
    assert_eq!(report.remaining, 3);
    assert_eq!(f.llm.structured_calls(), 4, "one call, not one per entry");
    let queue = f.db.pending_scoring().oldest(10).await.unwrap();
    assert!(
        queue.iter().all(|e| e.tries == 0),
        "a provider outage is not the entry's fault"
    );
}

#[tokio::test]
async fn an_entry_the_model_keeps_getting_wrong_is_given_up_on_for_review() {
    let f = fixture().await;
    let request = request_for(&f);
    f.llm
        .queue_structured(Err(LlmError::Timeout(TimeoutKind::Total)));
    f.scorer.score(request.clone(), &cancel()).await.unwrap();
    for pass in 1..=MAX_OUTPUT_TRIES {
        f.llm.queue_structured(Err(LlmError::Refusal));
        let report = f.scorer.score_pending_backlog(&cancel()).await.unwrap();
        if pass < MAX_OUTPUT_TRIES {
            assert_eq!((report.given_up, report.remaining), (0, 1), "pass {pass}");
        } else {
            assert_eq!((report.given_up, report.remaining), (1, 0));
        }
    }
    let rows = attempts(&f, &request).await;
    assert!(rows.iter().all(|a| a.status == AttemptStatus::NeedsReview));
    assert!(rows.iter().all(|a| !a.counts_toward_estimate));
    assert!(f.db.pending_scoring().oldest(10).await.unwrap().is_empty());
    let reasons = f.db.evidence().for_attempt(rows[0].id).await.unwrap();
    assert!(reasons.iter().any(|e| {
        e.kind == EvidenceKind::ScorerReason
            && e.content
                .as_deref()
                .is_some_and(|c| c.contains("needs review"))
    }));
}

#[tokio::test]
async fn the_backlog_leaves_other_services_entries_alone_and_skips_unreadable_ones() {
    let f = fixture().await;
    // A writing-workshop entry and a garbled entry of our kind.
    for (n, payload) in [
        json!({ "kind": "writing_draft", "n": 1 }),
        json!({ "kind": "rubric_response", "request": "not a request" }),
    ]
    .into_iter()
    .enumerate()
    {
        let attempt =
            f.db.attempts()
                .insert(&storage::NewAttempt {
                    profile_id: f.profile_id,
                    session_id: None,
                    unit_id: None,
                    activity_id: format!("x{n}"),
                    activity_type: "x".into(),
                    response_id: format!("resp-x{n}"),
                    origin: storage::AttemptOrigin::Authored,
                    level: storage::Level::A1,
                    skill: "writing".into(),
                    dimension: "task_achievement".into(),
                    scorer: storage::Scorer::RubricLlm,
                    scorer_version: "v".into(),
                    raw_score: None,
                    max_score: None,
                    normalized: None,
                    confidence: None,
                    status: AttemptStatus::PendingLlm,
                    counts_toward_estimate: true,
                    created_at: common::ts(1),
                })
                .await
                .unwrap();
        f.db.pending_scoring()
            .enqueue(attempt.id, &payload, &common::ts(2))
            .await
            .unwrap();
    }
    let report = f.scorer.score_pending_backlog(&cancel()).await.unwrap();
    assert_eq!(report.scored, 0);
    assert_eq!(report.skipped, 1);
    assert_eq!(f.llm.structured_calls(), 0);
    assert_eq!(f.db.pending_scoring().oldest(10).await.unwrap().len(), 2);
}

#[tokio::test]
async fn the_recorder_of_the_scorer_is_the_one_that_wrote_the_rows() {
    // The scorer exposes its recorder so a caller records responses of other
    // kinds (a deterministic score, a pronunciation report) in the same way.
    let f = fixture().await;
    let recorder: &EvidenceRecorder = f.scorer.recorder();
    let id = recorder.new_response_id("a10");
    assert!(id.starts_with("a10@"));
    assert_ne!(id, recorder.new_response_id("a10"));
}
