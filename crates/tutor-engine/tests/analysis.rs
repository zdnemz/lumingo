#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use assessment_engine::Level;
use common::{FakeLlm, make_profile, make_session, make_turn, temp_db, test_clock};
use llm_client::{LlmError, TimeoutKind};
use serde_json::{Value, json};
use storage::{Database, LlmCallType, LlmOutcome, SessionKind, TurnRole};
use tokio_util::sync::CancellationToken;
use tutor_engine::{
    AnalysisFailure, AnalysisKind, AnalyzerConfig, BATCH_EVERY, Cadence, InputMode, ObjectiveRef,
    TurnAnalyzer, TurnToAnalyse, analysis_unreliable,
};

struct Fixture {
    _dir: tempfile::TempDir,
    db: Database,
    llm: Arc<FakeLlm>,
    analyzer: TurnAnalyzer,
    profile_id: i64,
    session_id: i64,
}

async fn fixture(kind: AnalysisKind) -> Fixture {
    let (dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let session = make_session(&db, profile.id, SessionKind::TextChat).await;
    let llm = FakeLlm::new();
    let analyzer = TurnAnalyzer::new(
        AnalyzerConfig {
            profile_id: profile.id,
            session_id: session.id,
            provider_profile_id: None,
            model: "test-model".into(),
            level: Level::A2,
            first_language: "Indonesian".into(),
            kind,
            objectives: vec![ObjectiveRef {
                id: "o1".into(),
                can_do: "I can talk about my day.".into(),
            }],
            target_language: vec!["past simple".into()],
        },
        llm.clone(),
        db.clone(),
        test_clock(),
    );
    Fixture {
        _dir: dir,
        db,
        llm,
        analyzer,
        profile_id: profile.id,
        session_id: session.id,
    }
}

/// Stores a learner turn and returns the analysis input for it.
async fn learner_turn(f: &Fixture, text: &str) -> TurnToAnalyse {
    let turn = make_turn(&f.db, f.session_id, TurnRole::Learner, text).await;
    TurnToAnalyse {
        turn_id: turn.id,
        turn_seq: turn.seq,
        input_mode: InputMode::Text,
        tutor_before: "What did you do?".into(),
        learner_text: text.into(),
        tutor_reply: "Nice! Tell me more.".into(),
    }
}

fn error(category: &str, quote: &str, correction: &str, addressed: bool) -> Value {
    json!({
        "category": category, "quote": quote, "correction": correction,
        "severity": "major", "addressed_in_reply": addressed
    })
}

fn turn_reply(seq: i64, errors: Vec<Value>, note: &str) -> Value {
    json!({
        "turn_seq": seq, "errors": errors, "objective_evidence": [],
        "understood_tutor": "yes", "note_for_next_turn": note
    })
}

fn reply(turns: Vec<Value>) -> Value {
    json!({ "turns": turns })
}

fn cancel() -> CancellationToken {
    CancellationToken::new()
}

#[tokio::test]
async fn an_analysis_is_stored_with_its_events_stats_notes_and_call_log() {
    let f = fixture(AnalysisKind::Turn).await;
    let turn = learner_turn(&f, "Yesterday I go to a school and I eat a apple").await;
    f.llm.queue_structured(Ok(reply(vec![turn_reply(
        turn.turn_seq,
        vec![
            error("verb_tense", "I go to", "I went to", true),
            error("article", "a apple", "an apple", false),
            error("article", "invented words", "other words", false),
        ],
        "Practise a and an.",
    )])));

    let report = f
        .analyzer
        .turn_finished(turn.clone(), &cancel())
        .await
        .unwrap();

    assert_eq!(report.analysed.len(), 1);
    assert_eq!(report.failure, None);
    assert_eq!(report.waiting, 0);
    assert_eq!(report.analysed[0].dropped, 1);

    let stored =
        f.db.analysis()
            .get(turn.turn_id)
            .await
            .unwrap()
            .expect("analysis row");
    assert_eq!(stored.contract_version, "turn_analysis/1");
    assert_eq!(stored.model, "test-model");
    assert_eq!(stored.ladder_level, 1);
    assert_eq!(
        stored.analysis["turns"][0]["errors"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let events = f.db.analysis().error_events(turn.turn_id).await.unwrap();
    assert_eq!(events.len(), 2);
    assert!(
        events[0].addressed,
        "the tense error was addressed in the reply"
    );
    assert!(!events[1].addressed);

    let stats = f.db.error_stats().list(f.profile_id).await.unwrap();
    let counts: Vec<(&str, i64)> = stats
        .iter()
        .map(|s| (s.category.as_str(), s.count))
        .collect();
    assert_eq!(counts, [("article", 1), ("verb_tense", 1)]);

    assert_eq!(f.analyzer.notes(), ["Practise a and an."]);

    let calls = f.db.diagnostics().recent_llm_calls(5).await.unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].call_type, LlmCallType::TurnAnalysis);
    assert_eq!(calls[0].outcome, LlmOutcome::Ok);
    assert_eq!(calls[0].model, "test-model");
}

#[tokio::test]
async fn the_request_carries_the_stated_level_the_cap_and_no_level_from_a_model() {
    let f = fixture(AnalysisKind::Turn).await;
    let turn = learner_turn(&f, "I go home").await;
    f.llm
        .queue_structured(Ok(reply(vec![turn_reply(turn.turn_seq, vec![], "")])));
    f.analyzer.turn_finished(turn, &cancel()).await.unwrap();

    let seen = f.llm.structured_requests();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].contract, llm_client::Contract::TurnAnalysis);
    assert_eq!(seen[0].temperature, Some(0.0));
    assert!(seen[0].system.contains("At most 5 errors per turn"));
    let body: Value = serde_json::from_str(&seen[0].messages[0].content).unwrap();
    assert_eq!(body["level"], "A2");
    assert_eq!(body["l1"], "Indonesian");
    assert_eq!(body["input_mode"], "text");
    assert_eq!(body["turns"][0]["learner_text"], "I go home");
}

#[tokio::test]
async fn a_429_switches_to_batched_cadence_and_three_turns_go_in_one_call() {
    let f = fixture(AnalysisKind::Turn).await;
    let first = learner_turn(&f, "I go to school yesterday").await;
    f.llm
        .queue_structured(Err(LlmError::RateLimited { retry_after: None }));
    let report = f
        .analyzer
        .turn_finished(first.clone(), &cancel())
        .await
        .unwrap();
    assert_eq!(report.failure, Some(AnalysisFailure::RateLimited));
    assert_eq!(f.analyzer.cadence(), Cadence::Batched);
    assert_eq!(f.analyzer.waiting(), 1);
    assert!(f.db.analysis().get(first.turn_id).await.unwrap().is_none());
    assert_eq!(f.analyzer.flagged_turn_ids(), [first.turn_id]);

    let second = learner_turn(&f, "He go to market").await;
    let report = f
        .analyzer
        .turn_finished(second.clone(), &cancel())
        .await
        .unwrap();
    assert!(
        report.analysed.is_empty(),
        "two waiting turns are below the batch size"
    );
    assert_eq!(f.llm.structured_calls(), 1);

    let third = learner_turn(&f, "She like cats").await;
    f.llm.queue_structured(Ok(reply(vec![
        turn_reply(
            first.turn_seq,
            vec![error(
                "verb_tense",
                "I go to school",
                "I went to school",
                false,
            )],
            "",
        ),
        turn_reply(
            second.turn_seq,
            vec![error("subject_verb_agreement", "He go", "He goes", false)],
            "",
        ),
        turn_reply(
            third.turn_seq,
            vec![error(
                "subject_verb_agreement",
                "She like",
                "She likes",
                false,
            )],
            "Watch the -s.",
        ),
    ])));
    let report = f
        .analyzer
        .turn_finished(third.clone(), &cancel())
        .await
        .unwrap();

    assert_eq!(BATCH_EVERY, 3);
    assert_eq!(report.analysed.len(), 3);
    assert_eq!(f.llm.structured_calls(), 2, "one call for the three turns");
    assert_eq!(f.analyzer.waiting(), 0);
    assert!(f.analyzer.flagged_turn_ids().is_empty());
    let seen = f.llm.structured_requests();
    let body: Value = serde_json::from_str(&seen[1].messages[0].content).unwrap();
    assert_eq!(body["turns"].as_array().unwrap().len(), 3);

    let calls = f.db.diagnostics().recent_llm_calls(5).await.unwrap();
    assert!(calls.iter().any(|c| c.outcome == LlmOutcome::RateLimited));
}

#[tokio::test]
async fn flush_analyses_what_is_waiting_when_the_session_ends() {
    let f = fixture(AnalysisKind::Turn).await;
    f.llm.queue_structured(Err(LlmError::RateLimited {
        retry_after: Some(Duration::from_secs(2)),
    }));
    let a = learner_turn(&f, "I has a dog").await;
    f.analyzer
        .turn_finished(a.clone(), &cancel())
        .await
        .unwrap();
    let b = learner_turn(&f, "It are big").await;
    f.analyzer
        .turn_finished(b.clone(), &cancel())
        .await
        .unwrap();
    assert_eq!(f.analyzer.waiting(), 2);

    f.llm.queue_structured(Ok(reply(vec![
        turn_reply(
            a.turn_seq,
            vec![error("subject_verb_agreement", "I has", "I have", false)],
            "",
        ),
        turn_reply(
            b.turn_seq,
            vec![error("subject_verb_agreement", "It are", "It is", false)],
            "",
        ),
    ])));
    let report = f.analyzer.flush(&cancel()).await.unwrap();
    assert_eq!(report.analysed.len(), 2);
    assert_eq!(report.waiting, 0);
    assert!(f.db.analysis().get(a.turn_id).await.unwrap().is_some());
    assert!(f.db.analysis().get(b.turn_id).await.unwrap().is_some());
    assert!(f.analyzer.flagged_turn_ids().is_empty());
}

#[tokio::test]
async fn invalid_output_leaves_the_turn_stored_flagged_and_in_the_next_batch() {
    let f = fixture(AnalysisKind::Turn).await;
    let a = learner_turn(&f, "I has a dog").await;
    f.llm
        .queue_structured(Err(LlmError::InvalidOutput(llm_client::InvalidOutput {
            reason: llm_client::InvalidReason::SchemaMismatch,
            paths: vec![],
            ladder_level: llm_client::LadderLevel::NativeSchema,
            repaired: true,
        })));
    let report = f
        .analyzer
        .turn_finished(a.clone(), &cancel())
        .await
        .unwrap();
    assert_eq!(report.failure, Some(AnalysisFailure::InvalidOutput));
    assert_eq!(
        f.analyzer.cadence(),
        Cadence::Every,
        "bad output is not a 429"
    );
    assert!(
        f.db.turns().get(a.turn_id).await.unwrap().is_some(),
        "the turn is still stored"
    );
    assert!(f.db.analysis().get(a.turn_id).await.unwrap().is_none());
    assert_eq!(f.analyzer.flagged_turn_ids(), [a.turn_id]);

    let b = learner_turn(&f, "It are big").await;
    f.llm.queue_structured(Ok(reply(vec![
        turn_reply(
            a.turn_seq,
            vec![error("subject_verb_agreement", "I has", "I have", false)],
            "",
        ),
        turn_reply(b.turn_seq, vec![], ""),
    ])));
    let report = f.analyzer.turn_finished(b, &cancel()).await.unwrap();
    assert_eq!(report.analysed.len(), 2, "the flagged turn rode along");
    assert!(f.analyzer.flagged_turn_ids().is_empty());
    assert!(f.db.analysis().get(a.turn_id).await.unwrap().is_some());
}

#[tokio::test]
async fn a_turn_that_keeps_causing_bad_output_is_given_up_but_stays_flagged() {
    let f = fixture(AnalysisKind::Turn).await;
    let a = learner_turn(&f, "I has a dog").await;
    let b = learner_turn(&f, "It are big").await;
    let c = learner_turn(&f, "We is happy").await;
    for _ in 0..3 {
        f.llm
            .queue_structured(Err(LlmError::Protocol("garbled".into())));
    }
    // Each failed call re-sends the older turns, so `a` is in three batches.
    f.analyzer
        .turn_finished(a.clone(), &cancel())
        .await
        .unwrap();
    f.analyzer
        .turn_finished(b.clone(), &cancel())
        .await
        .unwrap();
    f.analyzer
        .turn_finished(c.clone(), &cancel())
        .await
        .unwrap();

    assert_eq!(f.analyzer.waiting(), 2, "a was given up");
    let flagged = f.analyzer.flagged_turn_ids();
    assert!(
        flagged.contains(&a.turn_id),
        "a given-up turn stays flagged"
    );
    assert!(f.db.analysis().get(a.turn_id).await.unwrap().is_none());
    assert!(f.db.turns().get(a.turn_id).await.unwrap().is_some());

    f.llm.queue_structured(Ok(reply(vec![
        turn_reply(b.turn_seq, vec![], ""),
        turn_reply(c.turn_seq, vec![], ""),
    ])));
    let report = f.analyzer.flush(&cancel()).await.unwrap();
    assert_eq!(report.analysed.len(), 2);
    assert_eq!(f.analyzer.flagged_turn_ids(), [a.turn_id]);
}

#[tokio::test]
async fn an_outage_keeps_the_turn_queued_without_counting_against_it() {
    let f = fixture(AnalysisKind::Turn).await;
    let a = learner_turn(&f, "I has a dog").await;
    for _ in 0..5 {
        f.llm
            .queue_structured(Err(LlmError::Timeout(TimeoutKind::Total)));
    }
    let report = f
        .analyzer
        .turn_finished(a.clone(), &cancel())
        .await
        .unwrap();
    assert_eq!(report.failure, Some(AnalysisFailure::ProviderUnavailable));
    for _ in 0..4 {
        f.analyzer.flush(&cancel()).await.unwrap();
    }
    assert_eq!(f.analyzer.waiting(), 1, "outages never give a turn up");
    assert_eq!(f.analyzer.cadence(), Cadence::Every);

    f.llm
        .queue_structured(Ok(reply(vec![turn_reply(a.turn_seq, vec![], "")])));
    let report = f.analyzer.flush(&cancel()).await.unwrap();
    assert_eq!(report.analysed.len(), 1);
}

#[tokio::test]
async fn a_reply_that_leaves_a_turn_out_flags_it() {
    let f = fixture(AnalysisKind::Turn).await;
    let a = learner_turn(&f, "I has a dog").await;
    let b = learner_turn(&f, "It are big").await;
    f.llm
        .queue_structured(Err(LlmError::RateLimited { retry_after: None }));
    f.analyzer
        .turn_finished(a.clone(), &cancel())
        .await
        .unwrap();
    f.analyzer
        .turn_finished(b.clone(), &cancel())
        .await
        .unwrap();
    f.llm
        .queue_structured(Ok(reply(vec![turn_reply(a.turn_seq, vec![], "")])));
    let report = f.analyzer.flush(&cancel()).await.unwrap();
    assert_eq!(report.analysed.len(), 1);
    assert!(f.analyzer.flagged_turn_ids().contains(&b.turn_id));
    assert!(!f.analyzer.flagged_turn_ids().contains(&a.turn_id));
}

#[tokio::test]
async fn a_turn_seq_that_was_not_sent_is_ignored() {
    let f = fixture(AnalysisKind::Turn).await;
    let a = learner_turn(&f, "I has a dog").await;
    f.llm.queue_structured(Ok(reply(vec![
        turn_reply(a.turn_seq, vec![], ""),
        turn_reply(999, vec![error("article", "a dog", "the dog", false)], ""),
    ])));
    let report = f.analyzer.turn_finished(a, &cancel()).await.unwrap();
    assert_eq!(report.analysed.len(), 1);
    assert!(
        f.db.error_stats()
            .list(f.profile_id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn notes_are_the_latest_three_newest_first() {
    let f = fixture(AnalysisKind::Turn).await;
    for (i, note) in ["one", "two", "three", "four"].into_iter().enumerate() {
        let t = learner_turn(&f, &format!("text number {i}")).await;
        f.llm
            .queue_structured(Ok(reply(vec![turn_reply(t.turn_seq, vec![], note)])));
        f.analyzer.turn_finished(t, &cancel()).await.unwrap();
    }
    assert_eq!(f.analyzer.notes(), ["four", "three", "two"]);
}

#[tokio::test]
async fn a_note_that_names_a_level_never_reaches_the_tutor() {
    let f = fixture(AnalysisKind::Turn).await;
    let t = learner_turn(&f, "I go home").await;
    f.llm.queue_structured(Ok(reply(vec![turn_reply(
        t.turn_seq,
        vec![],
        "This is B2 work.",
    )])));
    f.analyzer
        .turn_finished(t.clone(), &cancel())
        .await
        .unwrap();
    assert!(f.analyzer.notes().is_empty());
    let stored = f.db.analysis().get(t.turn_id).await.unwrap().unwrap();
    assert_eq!(stored.analysis["turns"][0]["note_for_next_turn"], "");
}

#[tokio::test]
async fn a_profile_whose_entries_keep_being_dropped_is_marked_unreliable() {
    let f = fixture(AnalysisKind::Turn).await;
    assert!(!analysis_unreliable(&f.db, None).await.unwrap());
    for i in 0..5 {
        let t = learner_turn(&f, &format!("plain text {i}")).await;
        f.llm.queue_structured(Ok(reply(vec![turn_reply(
            t.turn_seq,
            vec![error("article", "words that are not there", "other", false)],
            "",
        )])));
        f.analyzer.turn_finished(t, &cancel()).await.unwrap();
    }
    assert!(f.analyzer.unreliable());
    assert!(analysis_unreliable(&f.db, None).await.unwrap());
}

#[tokio::test]
async fn a_draft_is_analysed_at_once_with_the_draft_cap_and_text_mode() {
    let f = fixture(AnalysisKind::Draft).await;
    let words: Vec<String> = (0..25).map(|n| format!("wrd{n}")).collect();
    let text = words.join(" ");
    let turn = learner_turn(&f, &text).await;
    let turn = TurnToAnalyse {
        input_mode: InputMode::Voice,
        tutor_before: String::new(),
        tutor_reply: String::new(),
        ..turn
    };
    let errors: Vec<Value> = words
        .iter()
        .map(|w| error("word_choice", w, "other", false))
        .collect();
    f.llm
        .queue_structured(Ok(reply(vec![turn_reply(turn.turn_seq, errors, "")])));
    let record = f
        .analyzer
        .analyse_now(turn.clone(), &cancel())
        .await
        .unwrap();
    assert_eq!(record.analysis.errors.len(), 20);
    let seen = f.llm.structured_requests();
    assert!(seen[0].system.contains("At most 20 errors per turn"));
    let body: Value = serde_json::from_str(&seen[0].messages[0].content).unwrap();
    assert_eq!(body["input_mode"], "text", "a draft is always text");
    drop(seen);
    assert_eq!(
        f.db.analysis()
            .error_events(turn.turn_id)
            .await
            .unwrap()
            .len(),
        20
    );
}

#[tokio::test]
async fn analyse_now_reports_a_provider_failure_and_queues_nothing() {
    let f = fixture(AnalysisKind::Draft).await;
    let turn = learner_turn(&f, "Some draft text").await;
    f.llm
        .queue_structured(Err(LlmError::Transport(llm_client::TransportKind::Connect)));
    let result = f.analyzer.analyse_now(turn, &cancel()).await;
    assert!(matches!(result, Err(tutor_engine::EngineError::Llm(_))));
    assert_eq!(f.analyzer.waiting(), 0);
}

#[tokio::test]
async fn voice_turns_drop_spelling_errors_before_they_are_stored() {
    let f = fixture(AnalysisKind::Turn).await;
    let turn = learner_turn(&f, "my freind he go").await;
    let turn = TurnToAnalyse {
        input_mode: InputMode::Voice,
        ..turn
    };
    f.llm.queue_structured(Ok(reply(vec![turn_reply(
        turn.turn_seq,
        vec![
            error("spelling", "freind", "friend", false),
            error("verb_form", "he go", "he goes", false),
        ],
        "",
    )])));
    f.analyzer
        .turn_finished(turn.clone(), &cancel())
        .await
        .unwrap();
    let events = f.db.analysis().error_events(turn.turn_id).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].category, "verb_form");
}
