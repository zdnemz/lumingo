//! S4-12 writing workshop end to end: a `writing` session stores drafts as
//! turns, the first layer (rule findings) needs no provider, the second layer
//! (T2 errors) runs on the draft's text, a revision is classified against the
//! first draft as fixed / remaining / new, the drafts' attempts carry
//! `origin = free_mode` and are invisible to the level estimate, and a draft
//! with no provider comes back as a pending payload for the queue.
//!
//! A scripted `LlmClient` exists only here, in test code (AGENTS.md). The
//! rubric bands (T3) are S5-03's and are not part of this walk; ADR-051 records
//! that gap.
#![allow(clippy::unwrap_used)] // test helpers; clippy.toml only exempts #[test] functions

mod common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;

use assessment_engine::{
    Attempt as EstimateAttempt, Estimation, Level as EstimateLevel, Origin as EstimateOrigin,
    Skill as EstimateSkill, Status as EstimateStatus, estimate,
};
use common::TempDir;
use llm_client::{LlmError, StructuredOutput, StructuredRequest, TextRequest};
use storage::{
    AttemptOrigin, AttemptStatus, Database, EvidenceKind, InputMode, L1HelpMode, Level, NewAttempt,
    NewEvidence, NewProfile, NewSession, NewTurn, Scorer, SessionKind, SessionStatus, Skill,
    TurnRole, UiLanguage,
};
use tokio_util::sync::CancellationToken;
use tutor_engine::{
    DRAFT_PENDING_VERSION, DraftError, DraftRun, DraftSource, LlmClient, Resolution, RuleChecker,
    TextStream, WORKSHOP_ATTEMPT_SCORER_VERSION, Workshop, WorkshopConfig, compare_drafts,
};

const NOW: &str = "2026-10-08T08:00:00.000Z";

/// A scripted client: structured answers from a queue; streamed replies are not
/// part of the workshop, so `stream_text` always fails loudly.
struct Scripted {
    analyses: Mutex<Vec<Result<serde_json::Value, LlmError>>>,
    sent_structured: Mutex<Vec<StructuredRequest>>,
}

impl Scripted {
    fn new(analyses: Vec<Result<serde_json::Value, LlmError>>) -> Self {
        Self {
            analyses: Mutex::new(analyses),
            sent_structured: Mutex::new(Vec::new()),
        }
    }
}

impl LlmClient for Scripted {
    fn stream_text(
        &self,
        _request: TextRequest,
        _cancel: CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<TextStream, LlmError>> + Send + '_>> {
        Box::pin(async {
            Err(LlmError::Transport(
                "the workshop does not stream".to_owned(),
            ))
        })
    }

    fn structured(
        &self,
        request: StructuredRequest,
        _cancel: CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<StructuredOutput, LlmError>> + Send + '_>> {
        self.sent_structured.lock().unwrap().push(request);
        let answer = self.analyses.lock().unwrap().remove(0);
        Box::pin(async move {
            answer.map(|value| StructuredOutput {
                value,
                level: llm_client::Level::NativeSchema,
                repaired: false,
            })
        })
    }
}

/// One T2 answer for one draft: a fixed set of errors, in the contract's shape.
fn analysis(seq: i64, errors: &[(&str, &str, &str)]) -> serde_json::Value {
    let errors: Vec<serde_json::Value> = errors
        .iter()
        .map(|(quote, correction, category)| {
            serde_json::json!({
                "category": category,
                "quote": quote,
                "correction": correction,
                "severity": "major",
                "addressed_in_reply": false
            })
        })
        .collect();
    serde_json::json!({
        "turns": [{
            "turn_seq": seq,
            "errors": errors,
            "objective_evidence": [],
            "understood_tutor": "not_applicable",
            "note_for_next_turn": ""
        }]
    })
}

fn no_events(_event: tutor_engine::UiEvent) {}

fn workshop() -> Workshop {
    Workshop::start(WorkshopConfig {
        level: curriculum::Level::A1,
        first_language: "Indonesian".to_owned(),
        source: DraftSource::Topic("My weekend".to_owned()),
    })
    .unwrap()
}

/// The storage level of a curriculum level, at the call site (S4-02 split).
fn storage_level(level: curriculum::Level) -> Level {
    match level {
        curriculum::Level::A1 => Level::A1,
        curriculum::Level::A2 => Level::A2,
        curriculum::Level::B1 => Level::B1,
        curriculum::Level::B2 => Level::B2,
        curriculum::Level::C1 => Level::C1,
        curriculum::Level::C2 => Level::C2,
    }
}

#[tokio::test]
async fn two_drafts_are_stored_compared_and_never_count_toward_estimates() {
    let first_errors = [
        (
            "I has a book yesterday",
            "I had a book yesterday",
            "subject_verb_agreement",
        ),
        ("I see many fish", "I saw many fish", "verb_tense"),
        ("It was very happy", "I was very happy", "pronoun"),
    ];
    let second_errors = [
        ("I see many fish", "I saw many fish", "verb_tense"),
        ("The water was cold", "The water was cold.", "punctuation"),
    ];
    let client = Scripted::new(vec![
        Ok(analysis(1, &first_errors)),
        Ok(analysis(2, &second_errors)),
    ]);
    let mut workshop = workshop();
    let cancel = CancellationToken::new();

    // The database and the stored `writing` session.
    let dir = TempDir::new();
    let db = Database::open(dir.db_path()).await.unwrap();
    let profile = db
        .create_profile(NewProfile {
            display_name: "Learner".to_owned(),
            ui_language: UiLanguage::Id,
            l1: "id".to_owned(),
            l1_help_mode: L1HelpMode::Auto,
            created_at: NOW.to_owned(),
        })
        .await
        .unwrap();
    let session = db
        .create_session(NewSession {
            profile_id: profile.id,
            kind: SessionKind::Writing,
            unit_id: None,
            activity_id: None,
            mode: None,
            provider_profile_id: None,
            app_version: "0.0.0".to_owned(),
            started_at: NOW.to_owned(),
        })
        .await
        .unwrap();

    // The first draft: submit and the first feedback layer (the rule-based
    // findings) comes back at once, before any provider call; then the turn
    // and the attempt are stored, and the T2 analysis runs.
    let first_text = "I has a book yesterday. I see many fish. It was very happy.";
    let mut checker = RuleChecker::new();
    let submitted = workshop
        .submit_draft(first_text, &mut checker, &mut no_events)
        .unwrap();
    assert_eq!(submitted.learner_seq, 1);
    assert_eq!(submitted.words, 13);
    assert_eq!(submitted.rule.checked, cfg!(feature = "grammar"));
    if cfg!(feature = "grammar") {
        assert!(
            !submitted.rule.findings.is_empty(),
            "the checker flags the draft: {:?}",
            submitted.rule.findings
        );
    }
    let turn = db
        .add_turn(NewTurn {
            session_id: session.id,
            seq: submitted.learner_seq,
            role: TurnRole::Learner,
            input_mode: InputMode::Text,
            text: first_text.to_owned(),
            stt_text: None,
            edited_by_learner: false,
            speech_ms: None,
            pause_ms: None,
            word_count: Some(submitted.words as i64),
            created_at: NOW.to_owned(),
        })
        .await
        .unwrap();
    let described = workshop.attempt_for(session.id, turn.seq, submitted.words as i64);
    let attempt = db
        .add_attempt(NewAttempt {
            profile_id: profile.id,
            session_id: Some(session.id),
            unit_id: None,
            activity_id: described.activity_id,
            activity_type: described.activity_type,
            response_id: described.response_id,
            origin: AttemptOrigin::FreeMode,
            level: storage_level(curriculum::Level::A1),
            skill: Skill::Writing,
            dimension: described.dimension,
            scorer: Scorer::Deterministic,
            scorer_version: described.scorer_version,
            raw_score: None,
            max_score: None,
            normalized: None,
            confidence: None,
            status: AttemptStatus::Insufficient,
            counts_toward_estimate: false,
            created_at: NOW.to_owned(),
        })
        .await
        .unwrap();
    db.add_evidence(NewEvidence {
        attempt_id: attempt.id,
        kind: EvidenceKind::Metric,
        content: None,
        data_json: Some(serde_json::json!({ "words": described.words }).to_string()),
        created_at: NOW.to_owned(),
    })
    .await
    .unwrap();

    let run = workshop
        .analyse(&client, &cancel, &mut no_events)
        .await
        .unwrap();
    let DraftRun::Analysed {
        outcome,
        comparison,
    } = run
    else {
        panic!("the scripted client answers");
    };
    assert_eq!(outcome.filtered.turns[0].errors.len(), 3);
    assert!(
        comparison.is_none(),
        "the first draft has nothing to compare"
    );
    assert_eq!(outcome.filtered.counts.produced, 3);
    assert_eq!(outcome.filtered.counts.dropped, 0);

    // Store the analysis, as the chat path does.
    db.save_analysis_bundle(
        &storage::TurnAnalysis {
            turn_id: turn.id,
            analysis_json: outcome.filtered.analysis_json().unwrap(),
            contract_version: tutor_engine::TURN_ANALYSIS_VERSION.to_owned(),
            ladder_level: i64::from(outcome.ladder_level),
            model: "scripted-model".to_owned(),
            created_at: NOW.to_owned(),
        },
        &outcome.filtered.turns[0]
            .errors
            .iter()
            .map(|error| storage::NewErrorEvent {
                turn_id: turn.id,
                profile_id: profile.id,
                category: error.category.clone(),
                quote: error.quote.clone(),
                correction: error.correction.clone(),
                severity: match error.severity.as_str() {
                    "minor" => storage::Severity::Minor,
                    "major" => storage::Severity::Major,
                    _ => storage::Severity::Blocking,
                },
                addressed: error.addressed_in_reply,
                created_at: NOW.to_owned(),
            })
            .collect::<Vec<_>>(),
    )
    .await
    .unwrap();

    // The revision: a second draft at the next seq, stored the same way.
    let second_text = "I had a book yesterday. I see many fish. The water was cold.";
    let submitted = workshop
        .submit_draft(second_text, &mut checker, &mut no_events)
        .unwrap();
    assert_eq!(submitted.learner_seq, 2);
    if cfg!(feature = "grammar") {
        assert!(
            submitted.rule.findings.is_empty(),
            "the revision fixed the agreement mistake: {:?}",
            submitted.rule.findings
        );
    }
    db.add_turn(NewTurn {
        session_id: session.id,
        seq: submitted.learner_seq,
        role: TurnRole::Learner,
        input_mode: InputMode::Text,
        text: second_text.to_owned(),
        stt_text: None,
        edited_by_learner: false,
        speech_ms: None,
        pause_ms: None,
        word_count: Some(submitted.words as i64),
        created_at: NOW.to_owned(),
    })
    .await
    .unwrap();
    let described = workshop.attempt_for(session.id, submitted.learner_seq, submitted.words as i64);
    db.add_attempt(NewAttempt {
        profile_id: profile.id,
        session_id: Some(session.id),
        unit_id: None,
        activity_id: described.activity_id,
        activity_type: described.activity_type,
        response_id: described.response_id,
        origin: AttemptOrigin::FreeMode,
        level: storage_level(curriculum::Level::A1),
        skill: Skill::Writing,
        dimension: described.dimension,
        scorer: Scorer::Deterministic,
        scorer_version: described.scorer_version,
        raw_score: None,
        max_score: None,
        normalized: None,
        confidence: None,
        status: AttemptStatus::Insufficient,
        counts_toward_estimate: false,
        created_at: NOW.to_owned(),
    })
    .await
    .unwrap();

    let run = workshop
        .analyse(&client, &cancel, &mut no_events)
        .await
        .unwrap();
    let DraftRun::Analysed {
        outcome,
        comparison,
    } = run
    else {
        panic!("the scripted client answers");
    };
    let comparison = comparison.expect("the second draft is compared with the first");
    // "I has a book yesterday" and "It was very happy" are gone and not
    // reported again: fixed. "I see many fish" is reported again on words still
    // in the draft: remaining. The punctuation report is new.
    assert_eq!(comparison.fixed(), 2);
    assert_eq!(comparison.remaining(), 1);
    assert_eq!(comparison.new.len(), 1);
    assert_eq!(comparison.new[0].category, "punctuation");
    assert_eq!(
        comparison.earlier[1].error,
        DraftError {
            quote: "I see many fish".to_owned(),
            category: "verb_tense".to_owned(),
        }
    );
    assert_eq!(comparison.earlier[1].resolution, Resolution::Remaining);
    assert_eq!(outcome.filtered.turns[0].errors.len(), 2);

    // Both drafts are stored as turns; both attempts are free_mode and never
    // count, exactly as the chat path proves for its messages.
    let turns = db.turns_for_session(session.id).await.unwrap();
    assert_eq!(turns.len(), 2);
    assert!(turns.iter().all(|t| t.role == TurnRole::Learner));
    assert_eq!(turns[0].text, first_text);
    assert_eq!(turns[1].text, second_text);
    let rows = db
        .attempts_for_skill(profile.id, Skill::Writing)
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| row.origin == AttemptOrigin::FreeMode));
    assert!(rows.iter().all(|row| !row.counts_toward_estimate));
    assert!(
        rows.iter()
            .all(|row| row.scorer_version == WORKSHOP_ATTEMPT_SCORER_VERSION)
    );
    let estimator_rows: Vec<EstimateAttempt> = rows
        .iter()
        .map(|row| EstimateAttempt {
            response_id: 1,
            skill: EstimateSkill::Writing,
            level: EstimateLevel::A1,
            activity_id: 1,
            session_id: 1,
            scorer: assessment_engine::Scorer::Deterministic,
            normalized: 1.0,
            confidence: 1.0,
            status: EstimateStatus::Scored,
            origin: EstimateOrigin::FreeMode,
            counts_toward_estimate: row.counts_toward_estimate,
            created_at: 0,
        })
        .collect();
    let estimation: Estimation = estimate(&estimator_rows, &Default::default(), 0);
    let writing = estimation
        .estimates
        .iter()
        .find(|e| e.skill == EstimateSkill::Writing)
        .unwrap();
    assert_eq!(
        writing.status,
        assessment_engine::EstimateStatus::InsufficientEvidence
    );
    assert_eq!(writing.level, None);

    // The T2 calls were the draft's shape: text mode, empty tutor fields.
    let requests = client.sent_structured.lock().unwrap().clone();
    assert_eq!(requests.len(), 2);
    let body: serde_json::Value = serde_json::from_str(&requests[0].messages[0].content).unwrap();
    assert_eq!(body["input_mode"], "text");
    assert_eq!(body["turns"][0]["tutor_before"], "");
    assert_eq!(body["turns"][0]["tutor_reply"], "");
    assert_eq!(body["turns"][0]["learner_text"], first_text);
    // The draft cap of 20 errors is in the system prompt (T2).
    let system = requests[0].system.as_deref().unwrap();
    assert!(system.contains("At most 20 errors per turn"), "{system}");

    // Ending the session keeps every stored row.
    db.end_session(session.id, SessionStatus::Completed, NOW)
        .await
        .unwrap();
    assert_eq!(db.turns_for_session(session.id).await.unwrap().len(), 2);
}

#[tokio::test]
async fn a_draft_with_no_provider_waits_in_the_pending_queue() {
    let client = Scripted::new(vec![Err(LlmError::Transport("offline".to_owned()))]);
    let mut workshop = workshop();
    let cancel = CancellationToken::new();

    let dir = TempDir::new();
    let db = Database::open(dir.db_path()).await.unwrap();
    let profile = db
        .create_profile(NewProfile {
            display_name: "Learner".to_owned(),
            ui_language: UiLanguage::Id,
            l1: "id".to_owned(),
            l1_help_mode: L1HelpMode::Auto,
            created_at: NOW.to_owned(),
        })
        .await
        .unwrap();
    let session = db
        .create_session(NewSession {
            profile_id: profile.id,
            kind: SessionKind::Writing,
            unit_id: None,
            activity_id: None,
            mode: None,
            provider_profile_id: None,
            app_version: "0.0.0".to_owned(),
            started_at: NOW.to_owned(),
        })
        .await
        .unwrap();

    let text = "I go to school yesterday.";
    let mut checker = RuleChecker::new();
    let submitted = workshop
        .submit_draft(text, &mut checker, &mut no_events)
        .unwrap();
    let turn = db
        .add_turn(NewTurn {
            session_id: session.id,
            seq: submitted.learner_seq,
            role: TurnRole::Learner,
            input_mode: InputMode::Text,
            text: text.to_owned(),
            stt_text: None,
            edited_by_learner: false,
            speech_ms: None,
            pause_ms: None,
            word_count: Some(submitted.words as i64),
            created_at: NOW.to_owned(),
        })
        .await
        .unwrap();
    let described = workshop.attempt_for(session.id, turn.seq, submitted.words as i64);
    let attempt = db
        .add_attempt(NewAttempt {
            profile_id: profile.id,
            session_id: Some(session.id),
            unit_id: None,
            activity_id: described.activity_id,
            activity_type: described.activity_type,
            response_id: described.response_id,
            origin: AttemptOrigin::FreeMode,
            level: storage_level(curriculum::Level::A1),
            skill: Skill::Writing,
            dimension: described.dimension,
            scorer: Scorer::RubricLlm,
            scorer_version: described.scorer_version,
            raw_score: None,
            max_score: None,
            normalized: None,
            confidence: None,
            status: AttemptStatus::PendingLlm,
            counts_toward_estimate: false,
            created_at: NOW.to_owned(),
        })
        .await
        .unwrap();

    // The provider is unreachable: the draft comes back as a payload, the
    // session returns to waiting, and the caller queues the attempt (FR-W6).
    let run = workshop
        .analyse(&client, &cancel, &mut no_events)
        .await
        .unwrap();
    let DraftRun::Pending { payload, failure } = run else {
        panic!("the provider is unreachable");
    };
    assert!(matches!(
        failure,
        tutor_engine::AnalysisFailure::Provider(_)
    ));
    assert_eq!(payload.version, DRAFT_PENDING_VERSION);
    assert_eq!(payload.turn_seq, turn.seq);
    assert_eq!(payload.text, text);
    assert_eq!(payload.prompt.as_deref(), Some("My weekend"));
    assert!(payload.prompt_id.is_none());
    assert!(matches!(
        workshop.session().turn(),
        Some(tutor_engine::TurnState::Waiting)
    ));
    // The learner can keep revising while the feedback waits.
    let next = workshop.submit_draft("I went to school yesterday.", &mut checker, &mut no_events);
    assert!(next.is_ok());

    db.enqueue_pending_scoring(storage::NewPendingScoring {
        attempt_id: attempt.id,
        payload_json: serde_json::to_string(&payload).unwrap(),
        created_at: NOW.to_owned(),
    })
    .await
    .unwrap();
    let queue = db.pending_scoring(10).await.unwrap();
    assert_eq!(queue.len(), 1);
    let stored: serde_json::Value = serde_json::from_str(&queue[0].payload_json).unwrap();
    assert_eq!(stored["version"], "draft_pending/1");
    assert_eq!(stored["text"], text);
}

#[test]
fn the_comparison_helper_is_shared_with_the_module() {
    // The public helper and the module's own tests agree: one earlier error,
    // gone and not re-reported, is fixed.
    let first = [DraftError {
        quote: "I go".to_owned(),
        category: "verb_tense".to_owned(),
    }];
    let comparison = compare_drafts(&first, "I went.", &[]);
    assert_eq!(comparison.fixed(), 1);
    assert_eq!(comparison.remaining(), 0);
    assert!(comparison.new.is_empty());
}
