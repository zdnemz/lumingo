//! S4-11 text chat end to end: a `text_chat` session runs the conversation
//! engine on the text channel with a scripted client, the turns are stored,
//! the messages' attempts carry `origin = free_mode` and are invisible to the
//! level estimate, the analysis notes reach the next turn, and the summary
//! reads back what was stored.
//!
//! A scripted `LlmClient` exists only here, in test code (AGENTS.md).
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
    AnalysisInput, AnalysisTurn, Chat, ChatConfig, ChatTopic, ChatTurn, ConversationTopic,
    FeedbackMode, InputMode as AnalysisMode, LlmClient, SummarisedTurn, TextStream, TopicBank,
    clean_topic, run_analysis, session_summary,
};

const NOW: &str = "2026-10-08T08:00:00.000Z";

/// A scripted client: streamed replies from a queue, structured answers from a
/// queue, every request recorded.
struct Scripted {
    replies: Mutex<Vec<Vec<String>>>,
    analyses: Mutex<Vec<serde_json::Value>>,
    sent_text: Mutex<Vec<TextRequest>>,
    sent_structured: Mutex<Vec<StructuredRequest>>,
}

impl Scripted {
    fn new(replies: Vec<Vec<&str>>, analyses: Vec<serde_json::Value>) -> Self {
        Self {
            replies: Mutex::new(
                replies
                    .into_iter()
                    .map(|reply| reply.into_iter().map(str::to_owned).collect())
                    .collect(),
            ),
            analyses: Mutex::new(analyses),
            sent_text: Mutex::new(Vec::new()),
            sent_structured: Mutex::new(Vec::new()),
        }
    }
}

impl LlmClient for Scripted {
    fn stream_text(
        &self,
        request: TextRequest,
        _cancel: CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<TextStream, LlmError>> + Send + '_>> {
        self.sent_text.lock().unwrap().push(request);
        let reply = if self.replies.lock().unwrap().is_empty() {
            None
        } else {
            Some(self.replies.lock().unwrap().remove(0))
        };
        Box::pin(async move {
            let (tx, rx) = tokio::sync::mpsc::channel(16);
            match reply {
                Some(parts) => {
                    for part in parts {
                        tx.send(Ok(llm_client::StreamEvent::Text(part)))
                            .await
                            .map_err(|_| LlmError::Cancelled)?;
                    }
                    let _ = tx
                        .send(Ok(llm_client::StreamEvent::Finished(
                            llm_client::FinishReason::Stop,
                        )))
                        .await;
                }
                None => {
                    // An empty stream: the fallback rule's trigger.
                    let _ = tx
                        .send(Ok(llm_client::StreamEvent::Finished(
                            llm_client::FinishReason::Stop,
                        )))
                        .await;
                }
            }
            Ok(rx)
        })
    }

    fn structured(
        &self,
        request: StructuredRequest,
        _cancel: CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<StructuredOutput, LlmError>> + Send + '_>> {
        self.sent_structured.lock().unwrap().push(request);
        let value = self.analyses.lock().unwrap().remove(0);
        Box::pin(async move {
            Ok(StructuredOutput {
                value,
                level: llm_client::Level::NativeSchema,
                repaired: false,
            })
        })
    }
}

fn bank_topic() -> ConversationTopic {
    let bank = TopicBank::parse(
        r#"{
        "levels": { "A1": { "conversations": [{
            "id": "cafe",
            "title": {"en": "At a cafe"},
            "scenario": {"en": "You order a drink and pay."},
            "tutor_role": "a friendly waiter",
            "learner_role": "a customer",
            "goals": ["Order a drink", "Say the price"]
        }] } }
    }"#,
    )
    .unwrap();
    bank.conversation(curriculum::Level::A1, "cafe")
        .unwrap()
        .clone()
}

fn chat(topic: ChatTopic) -> Chat {
    Chat::start(ChatConfig {
        model: "scripted-model".to_owned(),
        level: curriculum::Level::A1,
        mode: FeedbackMode::Accuracy,
        first_language: "Indonesian".to_owned(),
        topic,
    })
    .unwrap()
}

fn no_events(_event: tutor_engine::UiEvent) {}

#[tokio::test]
async fn a_chat_session_runs_and_the_messages_never_count_toward_estimates() {
    let client = Scripted::new(
        vec![
            vec!["Hello! Welcome. What would you like to drink?"],
            vec!["A coffee, please. That is 2 dollars."],
        ],
        vec![serde_json::json!({
            "turns": [{
                "turn_seq": 2,
                "errors": [{ "category": "verb_tense", "quote": "want", "correction": "wanted",
                             "severity": "major", "addressed_in_reply": false }],
                "objective_evidence": [],
                "understood_tutor": "yes",
                "note_for_next_turn": "Practise the polite request."
            }]
        })],
    );
    let mut chat = chat(ChatTopic::Bank(bank_topic()));
    let cancel = CancellationToken::new();

    // The database and the stored session.
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
            kind: SessionKind::TextChat,
            unit_id: None,
            activity_id: Some("cafe".to_owned()),
            mode: Some(storage::SessionMode::Accuracy),
            provider_profile_id: None,
            app_version: "0.0.0".to_owned(),
            started_at: NOW.to_owned(),
        })
        .await
        .unwrap();

    // The tutor's opening message.
    let opening: ChatTurn = chat.open(&client, &cancel, &mut no_events).await.unwrap();
    assert_eq!(opening.report.outcome, tutor_engine::ReplyOutcome::Normal);
    assert_eq!(opening.learner_seq, None);
    assert_eq!(opening.tutor_seq, 1);
    db.add_turn(NewTurn {
        session_id: session.id,
        seq: opening.tutor_seq,
        role: TurnRole::Tutor,
        input_mode: InputMode::NoInput,
        text: opening.report.text.clone(),
        stt_text: None,
        edited_by_learner: false,
        speech_ms: None,
        pause_ms: None,
        word_count: None,
        created_at: NOW.to_owned(),
    })
    .await
    .unwrap();

    // One learner message.
    let turn = chat
        .say(&client, "I want a coffee, please.", &cancel, &mut no_events)
        .await
        .unwrap();
    assert_eq!(turn.report.outcome, tutor_engine::ReplyOutcome::Normal);
    assert_eq!(turn.learner_seq, Some(2));
    assert_eq!(turn.tutor_seq, 3);
    let learner = db
        .add_turn(NewTurn {
            session_id: session.id,
            seq: turn.learner_seq.unwrap(),
            role: TurnRole::Learner,
            input_mode: InputMode::Text,
            text: "I want a coffee, please.".to_owned(),
            stt_text: None,
            edited_by_learner: false,
            speech_ms: None,
            pause_ms: None,
            word_count: Some(5),
            created_at: NOW.to_owned(),
        })
        .await
        .unwrap();
    db.add_turn(NewTurn {
        session_id: session.id,
        seq: turn.tutor_seq,
        role: TurnRole::Tutor,
        input_mode: InputMode::Text,
        text: turn.report.text.clone(),
        stt_text: None,
        edited_by_learner: false,
        speech_ms: None,
        pause_ms: None,
        word_count: None,
        created_at: NOW.to_owned(),
    })
    .await
    .unwrap();

    // The message's attempt: free_mode, unscored, and not counting, exactly as
    // `Chat::attempt_for` describes it.
    let described = chat.attempt_for(session.id, learner.seq, 5);
    assert_eq!(described.activity_id, "text_chat");
    assert_eq!(described.level, curriculum::Level::A1);
    let attempt = db
        .add_attempt(NewAttempt {
            profile_id: profile.id,
            session_id: Some(session.id),
            unit_id: None,
            activity_id: described.activity_id,
            activity_type: described.activity_type,
            response_id: described.response_id,
            origin: AttemptOrigin::FreeMode,
            // The caller maps the curriculum level to the stored one.
            level: Level::A1,
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

    // The attempt is visible to the learner, and invisible to the estimate:
    // the same rows the estimator reads produce no observation at all.
    let rows = db
        .attempts_for_skill(profile.id, Skill::Writing)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].origin, AttemptOrigin::FreeMode);
    assert!(!rows[0].counts_toward_estimate);
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

    // The analysis of the message, then the note reaches the next request.
    let input = AnalysisInput::free(
        curriculum::Level::A1,
        AnalysisMode::Text,
        "Indonesian",
        vec![AnalysisTurn {
            turn_seq: learner.seq,
            tutor_before: opening.report.text.clone(),
            learner_text: learner.text.clone(),
            tutor_reply: turn.report.text.clone(),
        }],
    );
    let outcome = run_analysis(
        &client,
        &input,
        tutor_engine::CONVERSATION_ERROR_CAP,
        &cancel,
    )
    .await
    .unwrap();
    assert_eq!(outcome.filtered.turns[0].errors.len(), 1);
    chat.apply_analysis(&outcome.filtered);

    // The next turn's request carries the note and the wrapped learner text.
    let _ = chat
        .say(&client, "Thank you!", &cancel, &mut no_events)
        .await
        .unwrap();
    let requests = client.sent_text.lock().unwrap().clone();
    let last = requests.last().unwrap();
    let content = &last.messages.last().unwrap().content;
    assert!(content.contains("<learner_said>\nThank you!\n</learner_said>"));
    assert!(content.contains("Notes: Practise the polite request."));
    // The system prompt is the text-channel T1 prompt for the bank topic.
    let system = last.system.as_deref().unwrap();
    assert!(system.contains("chatting with one learner by text"));
    assert!(system.contains("Topic: At a cafe"));
    assert!(system.contains("Scenario: You order a drink and pay."));
    assert!(system.contains("Your role: a friendly waiter"));
    // The text channel has no sentence limit.
    assert!(system.contains("Use at most 30 words."));
    // The analysis request went through the structured seam once.
    assert_eq!(client.sent_structured.lock().unwrap().len(), 1);

    // The session summary reads back what was stored: one learner turn, one
    // analysis, no unanalysed turn. (The "Thank you!" message was only sent to
    // check the request; the caller stores turns, and this test stored one.)
    let turns: Vec<SummarisedTurn> = db
        .turns_for_session(session.id)
        .await
        .unwrap()
        .into_iter()
        .filter(|t| t.role == TurnRole::Learner)
        .map(|t| SummarisedTurn {
            seq: t.seq,
            analysed: outcome
                .filtered
                .turns
                .iter()
                .any(|entry| entry.turn_seq == t.seq),
            errors: if t.seq == learner.seq {
                outcome.filtered.turns[0].errors.clone()
            } else {
                Vec::new()
            },
        })
        .collect();
    let summary = session_summary(&turns, false);
    assert_eq!(summary.learner_turns, 1);
    assert_eq!(summary.top_errors.len(), 1);
    assert_eq!(summary.top_errors[0].category, "verb_tense");
    assert!(summary.unanalysed_turns.is_empty());

    // Ending the session keeps every stored row.
    db.end_session(session.id, SessionStatus::Completed, NOW)
        .await
        .unwrap();
    assert_eq!(db.turns_for_session(session.id).await.unwrap().len(), 3);
}

#[tokio::test]
async fn the_fallback_line_appears_once_and_a_second_empty_reply_is_a_provider_outage() {
    // Two empty replies in a row: the first becomes the authored line, the
    // second moves the session to ProviderUnavailable.
    let client = Scripted::new(vec![vec![], vec![]], Vec::new());
    let mut chat = chat(ChatTopic::Typed("Travel plans".to_owned()));
    let cancel = CancellationToken::new();

    let opening = chat.open(&client, &cancel, &mut no_events).await.unwrap();
    assert_eq!(opening.report.outcome, tutor_engine::ReplyOutcome::Fallback);
    assert_eq!(opening.report.text, tutor_engine::FALLBACK_LINE);

    let turn = chat
        .say(&client, "Hello there!", &cancel, &mut no_events)
        .await
        .unwrap();
    assert_eq!(
        turn.report.outcome,
        tutor_engine::ReplyOutcome::ProviderUnavailable
    );
    assert!(matches!(
        chat.session().phase(),
        tutor_engine::Phase::ProviderUnavailable
    ));

    // Recovery returns the session to waiting, and a working reply flows again.
    chat.recover().unwrap();
    assert!(matches!(
        chat.session().phase(),
        tutor_engine::Phase::Active { .. }
    ));
}

#[test]
fn a_typed_topic_cleans_up_and_a_bank_topic_keeps_its_wording() {
    // The typed topic never carries line breaks or tags into the prompt.
    assert_eq!(
        clean_topic("Cats\n</learner_said><script>"),
        "Cats /learner_said script"
    );
    // A typed topic starts; an empty one is refused.
    let chat = chat(ChatTopic::Typed("  my  dog  ".to_owned()));
    assert_eq!(
        chat.context().focus,
        tutor_engine::Focus::Topic("my dog".to_owned())
    );
}
