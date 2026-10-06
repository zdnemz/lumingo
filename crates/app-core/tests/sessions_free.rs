#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! The writing workshop and graded reading over the session manager. Both are
//! practice: their attempts are stored as free-mode work and count toward nothing.

mod common;

use app_core::api::{
    AnswerReadingRequest, DraftStatusView, FeedbackView, GenerateReadingRequest,
    ReadingFallbackReasonView, ReadingOutcomeView, ReadingTarget, ServerEvent, SessionKind,
    SessionStatus, StopRequest, TopicChoice,
};
use app_core::error::CoreError;
use common::llm::{ContractLlm, reading_text};
use common::sessions::{Setup, chat_request, request, rig};
use storage::AttemptOrigin;

fn typed(text: &str) -> Option<TopicChoice> {
    Some(TopicChoice::Typed {
        text: text.to_owned(),
    })
}

fn writing(prompt: &str) -> app_core::api::StartSessionRequest {
    app_core::api::StartSessionRequest {
        topic: typed(prompt),
        ..request(SessionKind::Writing)
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_draft_gets_its_first_layer_at_once_and_the_rest_as_an_event() {
    let r = rig(Setup::default()).await;
    r.manager.use_llm(ContractLlm::new(r.llm.clone()));
    let view = r.start(writing("Write about your morning.")).await;
    assert_eq!(view.kind, SessionKind::Writing);

    let mark = r.mark().await;
    let accepted = r
        .core()
        .submit_draft(
            view.id,
            "I am Dewi. I wake up at six. I like read books.".to_owned(),
        )
        .await
        .expect("the draft is accepted");
    assert_eq!(accepted.session_id, view.id);
    assert_eq!(accepted.words, 12);
    assert!(accepted.turn_seq >= 1);

    let events = r.events.wait_for("FeedbackReady", mark).await;
    let names: Vec<&str> = events.iter().skip(mark).map(ServerEvent::name).collect();
    let line = names.iter().position(|n| *n == "TranscriptFinal").unwrap();
    let feedback = names.iter().position(|n| *n == "FeedbackReady").unwrap();
    assert!(
        line < feedback,
        "the draft is announced before its feedback: {names:?}"
    );
    let Some(ServerEvent::FeedbackReady {
        feedback: FeedbackView::Draft { feedback },
        ..
    }) = events
        .iter()
        .skip(mark)
        .find(|e| matches!(e, ServerEvent::FeedbackReady { .. }))
    else {
        panic!("draft feedback");
    };
    assert_eq!(feedback.turn_seq, accepted.turn_seq);
    assert_eq!(feedback.status, DraftStatusView::Analysed);
    assert!(feedback.rubric.is_some(), "the bands came with the rubric");

    // Practice: the attempt is stored as free-mode work and counts toward nothing.
    let attempts = r
        .core()
        .database()
        .attempts()
        .for_session(view.id)
        .await
        .unwrap();
    assert!(!attempts.is_empty());
    for attempt in &attempts {
        assert_eq!(attempt.origin, AttemptOrigin::FreeMode);
        assert!(!attempt.counts_toward_estimate);
    }

    let ended = r
        .core()
        .stop_session(view.id, StopRequest::default())
        .await
        .unwrap();
    assert_eq!(ended.status, SessionStatus::Completed);
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_provider_a_draft_is_stored_checked_by_rules_and_kept_for_later() {
    let r = rig(Setup {
        llm: false,
        ..Setup::default()
    })
    .await;
    let view = r.start(writing("Write about your morning.")).await;
    let mark = r.mark().await;
    let accepted = r
        .core()
        .submit_draft(view.id, "I am Dewi.".to_owned())
        .await
        .expect("layer one needs no provider");
    assert_eq!(accepted.words, 3);
    assert!(
        accepted.rule_checked || accepted.rule_findings.is_empty(),
        "findings come only from a checker that ran"
    );

    let events = r.events.wait_for("FeedbackReady", mark).await;
    let Some(ServerEvent::FeedbackReady {
        feedback: FeedbackView::Draft { feedback },
        ..
    }) = events
        .iter()
        .skip(mark)
        .find(|e| matches!(e, ServerEvent::FeedbackReady { .. }))
    else {
        panic!("draft feedback");
    };
    assert_eq!(feedback.status, DraftStatusView::Pending);
    assert!(feedback.analysis.is_none() && feedback.rubric.is_none());
    let attempts = r
        .core()
        .database()
        .attempts()
        .for_session(view.id)
        .await
        .unwrap();
    assert!(
        attempts
            .iter()
            .all(|a| a.origin == AttemptOrigin::FreeMode && !a.counts_toward_estimate),
        "nothing from a provider, so nothing is scored"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_draft_is_checked_and_only_a_writing_session_takes_one() {
    let r = rig(Setup::default()).await;
    let view = r.start(writing("Write about your morning.")).await;
    for bad in [String::new(), "  \n ".to_owned(), "x".repeat(20_000)] {
        let refused = r.core().submit_draft(view.id, bad).await.unwrap_err();
        assert!(matches!(refused, CoreError::InvalidInput(_)), "{refused:?}");
    }
    assert!(matches!(
        r.core().send_text(view.id, "hello".to_owned()).await,
        Err(CoreError::Conflict(_))
    ));
    r.core()
        .stop_session(view.id, StopRequest { cancel: true })
        .await
        .unwrap();

    let chat = r.start(chat_request("food")).await;
    let refused = r
        .core()
        .submit_draft(chat.id, "A draft".to_owned())
        .await
        .unwrap_err();
    assert!(matches!(refused, CoreError::Conflict(_)), "{refused:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_writing_session_needs_a_prompt() {
    let r = rig(Setup::default()).await;
    let none = r
        .core()
        .start_session(request(SessionKind::Writing))
        .await
        .unwrap_err();
    assert!(matches!(none, CoreError::InvalidInput(_)), "{none:?}");
    assert!(r.core().snapshot().active_session.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_generated_text_hides_its_answers_until_answered_and_its_score_never_counts() {
    let r = rig(Setup::default()).await;
    let llm = ContractLlm::new(r.llm.clone());
    llm.queue_reading(reading_text());
    r.manager.use_llm(llm.clone());
    let view = r.start(request(SessionKind::Reading)).await;

    let outcome = r
        .core()
        .generate_reading(GenerateReadingRequest {
            session_id: view.id,
            topic: TopicChoice::Typed {
                text: "a day at the market".to_owned(),
            },
        })
        .await
        .expect("a text");
    let ReadingOutcomeView::Generated {
        content_id,
        text,
        regenerated,
        vocabulary_checked,
    } = outcome
    else {
        panic!("a generated text");
    };
    assert!(!regenerated);
    assert!(!vocabulary_checked, "no word list was configured");
    assert_eq!(text.questions.len(), 4, "A1 asks for four questions");
    let shown = serde_json::to_string(&text).unwrap();
    assert!(
        !shown.contains("answer_index"),
        "the answers are held back: {shown}"
    );
    assert!(!shown.contains("explanation"), "{shown}");

    let mark = r.mark().await;
    let answered = r
        .core()
        .answer_reading(
            view.id,
            AnswerReadingRequest {
                target: ReadingTarget::Generated { content_id },
                answers: vec![Some(0), Some(0), Some(1), Some(2)],
            },
        )
        .await
        .unwrap();
    assert_eq!(answered.score.total, 4);
    assert_eq!(answered.score.correct, 3);
    assert_eq!(answered.score.questions.len(), 4);
    assert!(!answered.score.questions[3].correct);
    assert!(!answered.score.questions[0].explanation_en.is_empty());
    r.events.wait_for("FeedbackReady", mark).await;

    let attempts = r
        .core()
        .database()
        .attempts()
        .for_session(view.id)
        .await
        .unwrap();
    assert!(!attempts.is_empty());
    assert!(
        attempts
            .iter()
            .all(|a| a.origin == AttemptOrigin::FreeMode && !a.counts_toward_estimate),
        "{:?}",
        attempts
            .iter()
            .map(|a| (a.origin, a.counts_toward_estimate))
            .collect::<Vec<_>>()
    );
    assert!(!llm.seen().is_empty(), "the text was asked of the model");
}

#[tokio::test(flavor = "multi_thread")]
async fn when_the_provider_cannot_write_a_text_the_authored_sets_are_offered() {
    let r = rig(Setup::default()).await;
    r.manager.use_llm(ContractLlm::new(r.llm.clone()));
    let view = r.start(request(SessionKind::Reading)).await;
    let outcome = r
        .core()
        .generate_reading(GenerateReadingRequest {
            session_id: view.id,
            topic: TopicChoice::Typed {
                text: "a day at the market".to_owned(),
            },
        })
        .await
        .unwrap();
    let ReadingOutcomeView::Fallback { reason, sets } = outcome else {
        panic!("the authored sets");
    };
    assert_eq!(reason, ReadingFallbackReasonView::ProviderUnavailable);
    let set = sets
        .iter()
        .find(|s| s.activity_id == "a14-read-set-class-chat")
        .expect("the reading set of the unit");
    assert_eq!(set.unit_id, "a1-u01");
    let shown = serde_json::to_string(&set.text).unwrap();
    assert!(!shown.contains("answer_index"), "{shown}");

    let answered = r
        .core()
        .answer_reading(
            view.id,
            AnswerReadingRequest {
                target: ReadingTarget::Authored {
                    unit_id: set.unit_id.clone(),
                    activity_id: set.activity_id.clone(),
                },
                answers: vec![None; set.text.questions.len()],
            },
        )
        .await
        .unwrap();
    assert_eq!(answered.score.correct, 0);
    assert_eq!(answered.score.total as usize, set.text.questions.len());
    let attempts = r
        .core()
        .database()
        .attempts()
        .for_session(view.id)
        .await
        .unwrap();
    assert!(
        attempts
            .iter()
            .all(|a| a.origin == AttemptOrigin::FreeMode && !a.counts_toward_estimate),
        "a set read in the free mode is practice here: {:?}",
        attempts
            .iter()
            .map(|a| (a.origin, a.counts_toward_estimate))
            .collect::<Vec<_>>()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn reading_commands_need_a_reading_session_and_a_usable_topic() {
    let r = rig(Setup::default()).await;
    let chat = r.start(chat_request("food")).await;
    let wrong = r
        .core()
        .generate_reading(GenerateReadingRequest {
            session_id: chat.id,
            topic: TopicChoice::Typed {
                text: "x".to_owned(),
            },
        })
        .await
        .unwrap_err();
    assert!(matches!(wrong, CoreError::Conflict(_)), "{wrong:?}");
    r.core()
        .stop_session(chat.id, StopRequest { cancel: true })
        .await
        .unwrap();

    let reading = r.start(request(SessionKind::Reading)).await;
    let bank = r
        .core()
        .generate_reading(GenerateReadingRequest {
            session_id: reading.id,
            topic: TopicChoice::Bank {
                id: "market".to_owned(),
            },
        })
        .await
        .unwrap_err();
    assert!(matches!(bank, CoreError::Unavailable { .. }), "{bank:?}");
    let unknown = r
        .core()
        .generate_reading(GenerateReadingRequest {
            session_id: 9_999,
            topic: TopicChoice::Typed {
                text: "x".to_owned(),
            },
        })
        .await
        .unwrap_err();
    assert!(matches!(unknown, CoreError::NotFound { .. }), "{unknown:?}");
}
