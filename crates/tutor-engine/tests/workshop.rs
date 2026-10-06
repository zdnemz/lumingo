#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod common;

use std::sync::Arc;

use assessment_engine::Level;
use common::{FakeLlm, make_profile, temp_db, test_clock};
use curriculum::validate::GrammarCheck;
use llm_client::{LlmError, TimeoutKind, TransportKind};
use serde_json::{Value, json};
use storage::{
    AttemptOrigin, AttemptStatus, Database, LlmCallType, Scorer, SessionStatus, TurnRole,
};
use tokio_util::sync::CancellationToken;
use tutor_engine::{
    Dimension, DraftStatus, Resolution, RubricDimension, Workshop, WorkshopConfig, WorkshopEnv,
    WorkshopRubric, WorkshopTask, process_pending_drafts,
};

/// A checker that flags two fixed mistakes. The real one is Harper, wired later.
struct FakeGrammar;

impl GrammarCheck for FakeGrammar {
    fn findings(&self, text: &str) -> Vec<String> {
        let mut out = Vec::new();
        if text.contains("teh") {
            out.push("Spelling: 'teh' may be 'the'".to_owned());
        }
        if text.contains("I has") {
            out.push("Grammar: use 'I have'".to_owned());
        }
        out
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    db: Database,
    llm: Arc<FakeLlm>,
    env: WorkshopEnv,
    workshop: Workshop,
}

fn rubric() -> WorkshopRubric {
    let dims = [
        Dimension::TaskAchievement,
        Dimension::Range,
        Dimension::Accuracy,
    ];
    WorkshopRubric {
        id: "w-a2".into(),
        version: 2,
        dimensions: dims
            .into_iter()
            .map(|dimension| RubricDimension {
                dimension,
                bands: ["nothing", "little", "some", "enough", "more"].map(String::from),
            })
            .collect(),
    }
}

fn task() -> WorkshopTask {
    WorkshopTask {
        prompt: "Write a note to a friend about the weekend.".into(),
        content_points: vec!["say when".into(), "say where".into()],
        min_words: Some(8),
    }
}

async fn fixture_with(rubric: Option<WorkshopRubric>) -> Fixture {
    let (dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let llm = FakeLlm::new();
    let env = WorkshopEnv {
        client: llm.clone(),
        db: db.clone(),
        clock: test_clock(),
        model: "test-model".into(),
        provider_profile_id: None,
        grammar: Some(Arc::new(FakeGrammar)),
        word_levels: None,
        provider_qualified: true,
    };
    let workshop = Workshop::start(
        env.clone(),
        WorkshopConfig {
            profile_id: profile.id,
            level: Level::A2,
            first_language: "Indonesian".into(),
            app_version: "0.0.0-test".into(),
            prompt_id: Some("note-friend".into()),
            rubric,
            task: task(),
        },
    )
    .await
    .unwrap();
    Fixture {
        _dir: dir,
        db,
        llm,
        env,
        workshop,
    }
}

async fn fixture() -> Fixture {
    fixture_with(Some(rubric())).await
}

fn cancel() -> CancellationToken {
    CancellationToken::new()
}

fn error(category: &str, quote: &str, correction: &str) -> Value {
    json!({ "category": category, "quote": quote, "correction": correction,
            "severity": "major", "addressed_in_reply": false })
}

fn t2(seq: i64, errors: Vec<Value>) -> Value {
    json!({ "turns": [{ "turn_seq": seq, "errors": errors, "objective_evidence": [],
        "understood_tutor": "not_applicable", "note_for_next_turn": "" }] })
}

fn t3(quote: &str) -> Value {
    json!({
        "dimension_scores": [
            { "dimension": "task_achievement", "band": "3", "evidence_quotes": [quote], "reason": "Covers the task." },
            { "dimension": "range", "band": "2", "evidence_quotes": [quote], "reason": "Simple words." },
            { "dimension": "accuracy", "band": "3", "evidence_quotes": [quote], "reason": "Mostly right." }
        ],
        "content_points": [
            { "point": "say when", "covered": true, "quote": quote },
            { "point": "say where", "covered": true, "quote": quote }
        ],
        "on_task": true,
        "feedback_en": "You gave clear plans. Add a reason next time.",
        "feedback_l1": "Rencanamu jelas. Tambahkan alasan lain kali."
    })
}

const DRAFT: &str =
    "Hi Sari, I has a plan for Saturday at the lake. We go to teh lake and swim together.";

#[tokio::test]
async fn rule_findings_come_at_once_with_no_provider_call_and_the_draft_is_stored() {
    let f = fixture().await;
    let submission = f.workshop.submit_draft(DRAFT).await.unwrap();
    assert_eq!(f.llm.structured_calls(), 0);
    assert!(submission.rule_checked);
    assert_eq!(submission.rule_findings.len(), 2);
    assert!(
        submission.rule_findings[0].contains("teh") || submission.rule_findings[1].contains("teh")
    );
    assert_eq!(submission.words, 19);
    assert!(!submission.below_minimum);

    let turns = f.db.turns().list(f.workshop.session_id()).await.unwrap();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].role, TurnRole::Learner);
    assert_eq!(turns[0].text, DRAFT);
    assert_eq!(turns[0].input_mode, storage::InputMode::Text);

    let short = f.workshop.submit_draft("Too short.").await.unwrap();
    assert!(short.below_minimum);
    assert!(f.workshop.submit_draft("  ").await.is_err());
}

#[tokio::test]
async fn a_reachable_provider_gives_errors_bands_and_stored_free_mode_attempts() {
    let f = fixture().await;
    let submission = f.workshop.submit_draft(DRAFT).await.unwrap();
    f.llm.queue_structured(Ok(t2(
        submission.turn_seq,
        vec![
            error("subject_verb_agreement", "I has", "I have"),
            error("spelling", "teh lake", "the lake"),
        ],
    )));
    f.llm.queue_structured(Ok(t3("Saturday at the lake")));
    let feedback = f
        .workshop
        .analyse_draft(&submission, &cancel())
        .await
        .unwrap();

    assert_eq!(feedback.status, DraftStatus::Analysed);
    let analysis = feedback.analysis.expect("layer two");
    assert_eq!(analysis.analysis.errors.len(), 2);
    assert!(feedback.comparison.is_none(), "there is no earlier draft");
    let rubric = feedback.rubric.expect("layer three");
    assert_eq!(rubric.dimensions.len(), 3);
    assert!(rubric.feedback_en.starts_with("You gave clear plans"));
    assert!(rubric.on_task);

    // T3 saw the rubric, the text input mode and the response.
    let seen = f.llm.structured_requests();
    assert_eq!(seen[0].contract, llm_client::Contract::TurnAnalysis);
    assert!(seen[0].system.contains("At most 20 errors per turn"));
    assert_eq!(seen[1].contract, llm_client::Contract::RubricScore);
    assert!(
        seen[1]
            .system
            .contains("Never state a CEFR level or a percentage.")
    );
    let body: Value = serde_json::from_str(&seen[1].messages[0].content).unwrap();
    assert_eq!(body["input_mode"], "text");
    assert_eq!(body["response"], DRAFT);
    assert_eq!(body["rubric"].as_array().unwrap().len(), 3);
    drop(seen);

    let attempts =
        f.db.attempts()
            .by_response(&format!("workshop-{}-1", f.workshop.session_id()))
            .await
            .unwrap();
    assert_eq!(attempts.len(), 3);
    let mut bands: Vec<(String, Option<f64>)> = attempts
        .iter()
        .map(|a| (a.dimension.clone(), a.normalized))
        .collect();
    bands.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        bands,
        [
            ("accuracy".to_owned(), Some(0.75)),
            ("range".to_owned(), Some(0.5)),
            ("task_achievement".to_owned(), Some(0.75))
        ]
    );
    for attempt in &attempts {
        assert_eq!(attempt.origin, AttemptOrigin::FreeMode);
        assert!(!attempt.counts_toward_estimate);
        assert_eq!(attempt.status, AttemptStatus::Scored);
        assert_eq!(attempt.scorer, Scorer::RubricLlm);
        assert_eq!(attempt.scorer_version, "w-a2/2+rubric_score/1+test-model");
        assert_eq!(attempt.confidence, Some(0.8));
        assert_eq!(attempt.skill, "writing");
    }
    let evidence = f.db.evidence().for_attempt(attempts[0].id).await.unwrap();
    assert!(
        evidence.iter().any(|e| e.content.as_deref() == Some(DRAFT)),
        "response text"
    );
    assert!(
        evidence
            .iter()
            .any(|e| e.content.as_deref() == Some("Saturday at the lake")),
        "quote"
    );

    let calls = f.db.diagnostics().recent_llm_calls(5).await.unwrap();
    let types: Vec<LlmCallType> = calls.iter().map(|c| c.call_type).collect();
    assert!(types.contains(&LlmCallType::TurnAnalysis));
    assert!(types.contains(&LlmCallType::RubricScore));
}

#[tokio::test]
async fn a_second_draft_is_classified_as_fixed_remaining_and_new() {
    let f = fixture().await;
    let first = f.workshop.submit_draft(DRAFT).await.unwrap();
    f.llm.queue_structured(Ok(t2(
        first.turn_seq,
        vec![
            error("subject_verb_agreement", "I has", "I have"),
            error("spelling", "teh lake", "the lake"),
        ],
    )));
    f.llm.queue_structured(Ok(t3("Saturday at the lake")));
    f.workshop.analyse_draft(&first, &cancel()).await.unwrap();

    let revised =
        "Hi Sari, I has a plan for Saturday at the lake. We goes to the lake and swim together.";
    let second = f.workshop.submit_draft(revised).await.unwrap();
    f.llm.queue_structured(Ok(t2(
        second.turn_seq,
        vec![
            error("subject_verb_agreement", "I has", "I have"),
            error("subject_verb_agreement", "We goes", "We go"),
        ],
    )));
    f.llm.queue_structured(Ok(t3("Saturday at the lake")));
    let feedback = f.workshop.analyse_draft(&second, &cancel()).await.unwrap();

    let comparison = feedback.comparison.expect("an earlier draft exists");
    let resolutions: Vec<(&str, Resolution)> = comparison
        .earlier
        .iter()
        .map(|e| (e.error.quote.as_str(), e.resolution))
        .collect();
    assert_eq!(
        resolutions,
        [
            ("I has", Resolution::Remaining),
            ("teh lake", Resolution::Fixed)
        ]
    );
    assert_eq!(comparison.new.len(), 1);
    assert_eq!(comparison.new[0].quote, "We goes");
}

#[tokio::test]
async fn with_no_provider_the_draft_waits_in_the_pending_queue() {
    let f = fixture().await;
    let submission = f.workshop.submit_draft(DRAFT).await.unwrap();
    f.llm
        .queue_structured(Err(LlmError::Transport(TransportKind::Connect)));
    let feedback = f
        .workshop
        .analyse_draft(&submission, &cancel())
        .await
        .unwrap();

    assert_eq!(feedback.status, DraftStatus::Pending);
    assert!(feedback.analysis.is_none() && feedback.rubric.is_none());
    assert!(
        f.db.turns()
            .get(submission.turn_id)
            .await
            .unwrap()
            .is_some(),
        "the draft is saved"
    );
    assert!(
        f.db.analysis()
            .get(submission.turn_id)
            .await
            .unwrap()
            .is_none()
    );
    let queue = f.db.pending_scoring().oldest(10).await.unwrap();
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0].payload["kind"], "writing_draft");
    assert_eq!(queue[0].payload["text"], DRAFT);
    let attempts =
        f.db.attempts()
            .for_session(f.workshop.session_id())
            .await
            .unwrap();
    assert_eq!(attempts.len(), 3);
    assert!(
        attempts
            .iter()
            .all(|a| a.status == AttemptStatus::PendingLlm && !a.counts_toward_estimate)
    );
    assert!(attempts.iter().all(|a| a.origin == AttemptOrigin::FreeMode));
}

#[tokio::test]
async fn a_pending_draft_is_finished_when_a_provider_is_back() {
    let f = fixture().await;
    let submission = f.workshop.submit_draft(DRAFT).await.unwrap();
    f.llm
        .queue_structured(Err(LlmError::Timeout(TimeoutKind::Total)));
    f.workshop
        .analyse_draft(&submission, &cancel())
        .await
        .unwrap();

    f.llm.queue_structured(Ok(t2(
        submission.turn_seq,
        vec![error("subject_verb_agreement", "I has", "I have")],
    )));
    f.llm.queue_structured(Ok(t3("Saturday at the lake")));
    let report = process_pending_drafts(&f.env, &cancel()).await.unwrap();

    assert_eq!(report.completed.len(), 1);
    assert_eq!(report.remaining, 0);
    let done = &report.completed[0];
    assert_eq!(done.session_id, f.workshop.session_id());
    assert_eq!(
        done.feedback
            .analysis
            .as_ref()
            .map(|a| a.analysis.errors.len()),
        Some(1)
    );
    assert!(done.feedback.rubric.is_some());
    assert!(f.db.pending_scoring().oldest(10).await.unwrap().is_empty());
    assert!(
        f.db.analysis()
            .get(submission.turn_id)
            .await
            .unwrap()
            .is_some()
    );
    let attempts =
        f.db.attempts()
            .for_session(f.workshop.session_id())
            .await
            .unwrap();
    assert_eq!(
        attempts.len(),
        3,
        "the placeholders were updated, not duplicated"
    );
    assert!(
        attempts
            .iter()
            .all(|a| a.status == AttemptStatus::Scored && a.normalized.is_some())
    );
    assert!(attempts.iter().all(|a| !a.counts_toward_estimate));
}

#[tokio::test]
async fn a_provider_that_is_still_away_is_asked_once_and_the_draft_stays_queued() {
    let f = fixture().await;
    for text in [
        DRAFT,
        "I like to swim in the lake with my friends on Sunday.",
    ] {
        let s = f.workshop.submit_draft(text).await.unwrap();
        f.llm
            .queue_structured(Err(LlmError::Transport(TransportKind::Connect)));
        f.workshop.analyse_draft(&s, &cancel()).await.unwrap();
    }
    f.llm
        .queue_structured(Err(LlmError::Transport(TransportKind::Connect)));
    let before = f.llm.structured_calls();
    let report = process_pending_drafts(&f.env, &cancel()).await.unwrap();
    assert!(report.completed.is_empty());
    assert_eq!(report.remaining, 2);
    assert_eq!(
        f.llm.structured_calls(),
        before + 1,
        "no second call against a failing provider"
    );
    let queue = f.db.pending_scoring().oldest(10).await.unwrap();
    assert_eq!(queue[0].tries, 1);
    assert_eq!(queue[1].tries, 0);
}

#[tokio::test]
async fn when_only_the_rubric_fails_the_errors_are_kept_and_only_the_rubric_is_retried() {
    let f = fixture().await;
    let submission = f.workshop.submit_draft(DRAFT).await.unwrap();
    f.llm.queue_structured(Ok(t2(
        submission.turn_seq,
        vec![error("subject_verb_agreement", "I has", "I have")],
    )));
    f.llm
        .queue_structured(Err(LlmError::Timeout(TimeoutKind::Total)));
    let feedback = f
        .workshop
        .analyse_draft(&submission, &cancel())
        .await
        .unwrap();
    assert_eq!(feedback.status, DraftStatus::RubricPending);
    assert!(feedback.analysis.is_some());
    assert!(feedback.rubric.is_none());
    assert!(
        f.db.analysis()
            .get(submission.turn_id)
            .await
            .unwrap()
            .is_some()
    );

    f.llm.queue_structured(Ok(t3("Saturday at the lake")));
    let calls_before = f.llm.structured_calls();
    let report = process_pending_drafts(&f.env, &cancel()).await.unwrap();
    assert_eq!(
        f.llm.structured_calls(),
        calls_before + 1,
        "T2 is not run again"
    );
    assert_eq!(report.completed.len(), 1);
    assert!(
        report.completed[0].feedback.analysis.is_none(),
        "already stored earlier"
    );
    assert!(report.completed[0].feedback.rubric.is_some());
    assert_eq!(
        f.db.analysis()
            .error_events(submission.turn_id)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn evidence_that_does_not_hold_causes_one_rerun_of_the_rubric() {
    let f = fixture().await;
    let submission = f.workshop.submit_draft(DRAFT).await.unwrap();
    f.llm.queue_structured(Ok(t2(submission.turn_seq, vec![])));
    let mut bad = t3("Saturday at the lake");
    bad["dimension_scores"][1]["evidence_quotes"] = json!(["words the learner never wrote"]);
    f.llm.queue_structured(Ok(bad));
    f.llm.queue_structured(Ok(t3("Saturday at the lake")));
    let feedback = f
        .workshop
        .analyse_draft(&submission, &cancel())
        .await
        .unwrap();
    let rubric = feedback.rubric.unwrap();
    assert_eq!(f.llm.structured_calls(), 3);
    assert!(rubric.rejected.is_empty());
    assert!(rubric.dimensions.iter().all(|d| d.band.is_some()));
}

#[tokio::test]
async fn evidence_that_fails_twice_leaves_the_dimension_for_review() {
    let f = fixture().await;
    let submission = f.workshop.submit_draft(DRAFT).await.unwrap();
    f.llm.queue_structured(Ok(t2(submission.turn_seq, vec![])));
    for _ in 0..2 {
        let mut bad = t3("Saturday at the lake");
        bad["dimension_scores"][1]["evidence_quotes"] = json!(["words the learner never wrote"]);
        f.llm.queue_structured(Ok(bad));
    }
    f.workshop
        .analyse_draft(&submission, &cancel())
        .await
        .unwrap();
    assert_eq!(f.llm.structured_calls(), 3, "one rerun, not more");
    let attempts =
        f.db.attempts()
            .for_session(f.workshop.session_id())
            .await
            .unwrap();
    let range = attempts.iter().find(|a| a.dimension == "range").unwrap();
    assert_eq!(range.status, AttemptStatus::NeedsReview);
    assert_eq!(range.normalized, None);
}

#[tokio::test]
async fn an_off_task_draft_gets_band_zero_in_every_dimension() {
    let f = fixture().await;
    let submission = f.workshop.submit_draft(DRAFT).await.unwrap();
    f.llm.queue_structured(Ok(t2(submission.turn_seq, vec![])));
    let mut off = t3("Saturday at the lake");
    off["on_task"] = json!(false);
    f.llm.queue_structured(Ok(off));
    let feedback = f
        .workshop
        .analyse_draft(&submission, &cancel())
        .await
        .unwrap();
    let rubric = feedback.rubric.unwrap();
    assert!(!rubric.on_task);
    assert!(rubric.dimensions.iter().all(|d| d.band == Some(0)));
}

#[tokio::test]
async fn without_a_rubric_the_workshop_gives_layers_one_and_two_only() {
    let f = fixture_with(None).await;
    let submission = f.workshop.submit_draft(DRAFT).await.unwrap();
    f.llm.queue_structured(Ok(t2(
        submission.turn_seq,
        vec![error("spelling", "teh lake", "the lake")],
    )));
    let feedback = f
        .workshop
        .analyse_draft(&submission, &cancel())
        .await
        .unwrap();
    assert_eq!(feedback.status, DraftStatus::Analysed);
    assert!(feedback.rubric.is_none());
    assert_eq!(f.llm.structured_calls(), 1);
    assert!(
        f.db.attempts()
            .for_session(f.workshop.session_id())
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn without_a_rubric_a_queued_draft_finishes_with_errors_and_an_unscored_placeholder() {
    let f = fixture_with(None).await;
    let submission = f.workshop.submit_draft(DRAFT).await.unwrap();
    f.llm
        .queue_structured(Err(LlmError::Transport(TransportKind::Connect)));
    f.workshop
        .analyse_draft(&submission, &cancel())
        .await
        .unwrap();
    f.llm.queue_structured(Ok(t2(
        submission.turn_seq,
        vec![error("spelling", "teh lake", "the lake")],
    )));
    let report = process_pending_drafts(&f.env, &cancel()).await.unwrap();
    assert_eq!(report.completed.len(), 1);
    let attempts =
        f.db.attempts()
            .for_session(f.workshop.session_id())
            .await
            .unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].status, AttemptStatus::Insufficient);
    assert!(!attempts[0].counts_toward_estimate);
}

#[tokio::test]
async fn a_cancelled_analysis_is_an_error_and_the_draft_is_not_lost() {
    let f = fixture().await;
    let submission = f.workshop.submit_draft(DRAFT).await.unwrap();
    f.llm.queue_structured(Ok(t2(submission.turn_seq, vec![])));
    f.llm.queue_structured(Err(LlmError::Cancelled));
    let result = f.workshop.analyse_draft(&submission, &cancel()).await;
    assert!(matches!(
        result,
        Err(tutor_engine::EngineError::Llm(LlmError::Cancelled))
    ));
    assert_eq!(
        f.db.pending_scoring().oldest(10).await.unwrap().len(),
        1,
        "queued for later"
    );
}

#[tokio::test]
async fn finishing_records_how_many_drafts_still_wait_for_feedback() {
    let f = fixture().await;
    let s = f.workshop.submit_draft(DRAFT).await.unwrap();
    f.llm
        .queue_structured(Err(LlmError::Transport(TransportKind::Connect)));
    f.workshop.analyse_draft(&s, &cancel()).await.unwrap();
    f.workshop.finish(false).await.unwrap();
    let session =
        f.db.sessions()
            .get(f.workshop.session_id())
            .await
            .unwrap()
            .unwrap();
    assert_eq!(session.status, SessionStatus::Completed);
    let summary = session.summary.unwrap();
    assert_eq!(summary["drafts"], 1);
    assert_eq!(summary["awaiting_feedback"], 1);
    assert_eq!(session.activity_id.as_deref(), Some("note-friend"));
}

#[tokio::test]
async fn deleting_the_session_removes_drafts_analysis_and_the_queue() {
    let f = fixture().await;
    let s = f.workshop.submit_draft(DRAFT).await.unwrap();
    f.llm
        .queue_structured(Err(LlmError::Transport(TransportKind::Connect)));
    f.workshop.analyse_draft(&s, &cancel()).await.unwrap();
    f.db.sessions()
        .delete(f.workshop.session_id())
        .await
        .unwrap();
    assert!(f.db.pending_scoring().oldest(10).await.unwrap().is_empty());
    assert!(f.db.turns().get(s.turn_id).await.unwrap().is_none());
}

#[tokio::test]
async fn without_a_checker_the_first_layer_says_it_did_not_check_and_x5_is_not_evaluated() {
    let f = fixture().await;
    let mut env = f.env.clone();
    env.grammar = None;
    let workshop = Workshop::start(
        env,
        WorkshopConfig {
            profile_id: 1,
            level: Level::A2,
            first_language: "Indonesian".into(),
            app_version: "0.0.0-test".into(),
            prompt_id: None,
            rubric: Some(rubric()),
            task: task(),
        },
    )
    .await
    .unwrap();
    let submission = workshop.submit_draft(DRAFT).await.unwrap();
    assert!(!submission.rule_checked, "nothing was checked");
    assert!(submission.rule_findings.is_empty());
    f.llm.queue_structured(Ok(t2(submission.turn_seq, vec![])));
    let mut reply = t3("Saturday at the lake");
    reply["dimension_scores"][2]["band"] = json!("4");
    f.llm.queue_structured(Ok(reply));
    let feedback = workshop
        .analyse_draft(&submission, &cancel())
        .await
        .unwrap();
    let rubric = feedback.rubric.expect("layer three");
    assert!(rubric.alarms.is_empty(), "no checker, no X5 alarm");
}
