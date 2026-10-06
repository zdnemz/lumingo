#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! The session manager over a text chat: the one-session rule, the order of the
//! events of a turn, pause and resume, the snapshot of a running session.

mod common;

use std::sync::Arc;
use std::time::Duration;

use app_core::api::{
    EngineId, EngineState, ErrorCode, Feature, FeedbackView, LineRole, ServerEvent, SessionKind,
    SessionLife, SessionStatus, StopRequest, TurnPhase,
};
use app_core::error::CoreError;
use app_core::voice::testing::{ReplyScript, Step};
use common::sessions::{Setup, chat_request, request, rig, unit_request};
use tokio::sync::Semaphore;

fn reply(text: &str) -> Step {
    Step::Reply(ReplyScript::new(&[text]))
}

/// The names of the events that came after position `from`, without the ones that
/// are not about the turn (heartbeats and engine status).
fn names_after(events: &[ServerEvent], from: usize) -> Vec<&'static str> {
    events
        .iter()
        .skip(from)
        .map(ServerEvent::name)
        .filter(|n| !matches!(*n, "Heartbeat" | "EngineStatus"))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_text_chat_turn_sends_its_events_in_a_defined_order() {
    let r = rig(Setup {
        steps: vec![
            Step::Reply(ReplyScript::new(&["Hello! ", "What is your name?"])),
            Step::Reply(ReplyScript::new(&["Nice to ", "meet you, ", "Dewi."])),
        ],
        ..Setup::default()
    })
    .await;
    let view = r.start(chat_request("introductions")).await;
    assert_eq!(view.kind, SessionKind::TextChat);
    assert_eq!(view.life, SessionLife::Active);

    // The first event of a session is its state, then the tutor speaks first.
    r.events
        .wait("the opening turn to end", |events| {
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
    let opening = r.events.all();
    assert!(
        matches!(
            opening.first(),
            Some(ServerEvent::SessionState {
                life: SessionLife::Active,
                kind: SessionKind::TextChat,
                ..
            })
        ),
        "the first event is the session becoming active: {:?}",
        names_after(&opening, 0)
    );
    assert_eq!(
        names_after(&opening, 0),
        [
            "SessionState",
            "TurnState",
            "TurnState",
            "TutorTextDelta",
            "TutorTextDelta",
            "TurnState"
        ],
        "opening: active, thinking, replying, two pieces, waiting"
    );

    let mark = r.mark().await;
    let accepted = r
        .core()
        .send_text(view.id, "My name is Dewi.".to_owned())
        .await
        .expect("the turn is accepted");
    assert_eq!(accepted.session_id, view.id);
    assert_eq!(accepted.turn, 2, "the opening was turn 1");
    let events = r.events.wait_for("AnalysisReady", mark).await;

    assert_eq!(
        names_after(&events, mark),
        [
            "TranscriptFinal",
            "TurnState",
            "TurnState",
            "TutorTextDelta",
            "TutorTextDelta",
            "TutorTextDelta",
            "TurnState",
            "AnalysisReady"
        ],
        "line, thinking, replying, three pieces, waiting, then the analysis"
    );
    let after: Vec<&ServerEvent> = events.iter().skip(mark).collect();
    let ServerEvent::TranscriptFinal {
        text,
        turn,
        turn_seq: line_seq,
        ..
    } = after[0]
    else {
        panic!("the learner's line comes first");
    };
    assert_eq!(text, "My name is Dewi.");
    assert_eq!(*turn, 2);
    let ServerEvent::TurnState {
        state,
        tutor_turn_seq,
        ..
    } = after[6]
    else {
        panic!("the reply ends the turn");
    };
    assert_eq!(*state, TurnPhase::Waiting);
    assert!(tutor_turn_seq.is_some(), "the stored position of the reply");
    let ServerEvent::AnalysisReady { analysis, .. } = after[7] else {
        panic!("the analysis comes last");
    };
    assert_eq!(Some(analysis.turn_seq), *line_seq, "it is about that line");

    let deltas: String = after
        .iter()
        .filter_map(|e| match e {
            ServerEvent::TutorTextDelta { delta, .. } => Some(delta.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(deltas, "Nice to meet you, Dewi.");

    // Every event of the run has the next number: no gaps, none repeated.
    let all = r.events.settled().await;
    for pair in all.windows(2) {
        assert_eq!(pair[1].seq(), pair[0].seq() + 1, "{:?}", r.events.names());
    }
    assert_eq!(r.core().events().current_seq(), all.last().unwrap().seq());
}

#[tokio::test(flavor = "multi_thread")]
async fn stopping_a_chat_sends_its_summary_before_the_session_ends() {
    let r = rig(Setup {
        steps: vec![
            reply("Hello. What is your name?"),
            reply("Nice to meet you."),
        ],
        ..Setup::default()
    })
    .await;
    let view = r.start(chat_request("introductions")).await;
    r.events.reply_stored(0).await;
    r.core()
        .send_text(view.id, "I am Dewi.".to_owned())
        .await
        .unwrap();
    r.events.wait_for("AnalysisReady", 0).await;

    let mark = r.mark().await;
    let ended = r
        .core()
        .stop_session(view.id, StopRequest::default())
        .await
        .expect("stops");
    assert_eq!(ended.status, SessionStatus::Completed);
    assert_eq!(ended.kind, SessionKind::TextChat);
    assert!(matches!(
        ended.feedback,
        Some(FeedbackView::Conversation { .. })
    ));
    let events = r.events.settled().await;
    assert_eq!(
        names_after(&events, mark),
        ["FeedbackReady", "SessionState"],
        "the summary first, then the end"
    );
    assert!(matches!(
        events.last(),
        Some(ServerEvent::SessionState {
            life: SessionLife::Ended,
            ..
        })
    ));
    assert!(r.core().snapshot().active_session.is_none());
    let stored = r
        .core()
        .database()
        .sessions()
        .get(view.id)
        .await
        .unwrap()
        .expect("stored");
    assert_eq!(stored.status, storage::SessionStatus::Completed);
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_a_chat_marks_it_aborted_and_makes_no_summary() {
    let r = rig(Setup {
        steps: vec![reply("Hello.")],
        ..Setup::default()
    })
    .await;
    let view = r.start(chat_request("food")).await;
    r.events.reply_stored(0).await;
    let ended = r
        .core()
        .stop_session(view.id, StopRequest { cancel: true })
        .await
        .unwrap();
    assert_eq!(ended.status, SessionStatus::Aborted);
    assert!(ended.feedback.is_none());
    assert!(!r.events.names().contains(&"FeedbackReady"));
    let stored = r
        .core()
        .database()
        .sessions()
        .get(view.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, storage::SessionStatus::Aborted);
}

#[tokio::test(flavor = "multi_thread")]
async fn only_one_session_runs_at_a_time() {
    let r = rig(Setup {
        steps: vec![reply("Hello.")],
        ..Setup::default()
    })
    .await;
    let first = r.start(chat_request("food")).await;
    for second in [
        chat_request("music"),
        unit_request(SessionKind::Lesson, "a1-u01"),
        request(SessionKind::Reading),
    ] {
        let refused = r.core().start_session(second).await.unwrap_err();
        assert!(
            matches!(refused, CoreError::Conflict(_)),
            "a second start while one runs is a conflict: {refused:?}"
        );
    }
    assert_eq!(r.core().snapshot().active_session.unwrap().id, first.id);

    // Stopping gives the slot back.
    r.core()
        .stop_session(first.id, StopRequest { cancel: true })
        .await
        .unwrap();
    let again = r.start(request(SessionKind::Reading)).await;
    assert_ne!(again.id, first.id);
}

#[tokio::test(flavor = "multi_thread")]
async fn two_starts_at_the_same_moment_give_one_session_and_one_refusal() {
    let r = rig(Setup {
        steps: vec![reply("Hello."), reply("Hi.")],
        ..Setup::default()
    })
    .await;
    let core_a = Arc::clone(r.core());
    let core_b = Arc::clone(r.core());
    let (a, b) = tokio::join!(
        async move { core_a.start_session(chat_request("food")).await },
        async move { core_b.start_session(chat_request("music")).await },
    );
    let results = [a, b];
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, Err(CoreError::Conflict(_))))
            .count(),
        1
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn commands_for_a_session_that_is_not_running_say_so() {
    let r = rig(Setup::default()).await;
    assert!(matches!(
        r.core().send_text(99, "hi".to_owned()).await,
        Err(CoreError::NotFound { what: "session" })
    ));
    assert!(matches!(
        r.core().stop_session(99, StopRequest::default()).await,
        Err(CoreError::NotFound { what: "session" })
    ));
    assert!(matches!(
        r.core().pause_session(99).await,
        Err(CoreError::NotFound { .. })
    ));
    assert!(matches!(
        r.core().resume_session(99).await,
        Err(CoreError::NotFound { .. })
    ));
    assert!(matches!(
        r.core().edit_turn(99, 1, "x".to_owned()).await,
        Err(CoreError::NotFound { .. })
    ));
    assert!(matches!(
        r.core().next_activity(99).await,
        Err(CoreError::NotFound { .. })
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_is_checked_and_a_chat_takes_no_transcript_edit_or_push_to_talk() {
    let r = rig(Setup {
        steps: vec![reply("Hello.")],
        ..Setup::default()
    })
    .await;
    let view = r.start(chat_request("food")).await;
    r.events.reply_stored(0).await;
    for bad in [String::new(), "   ".to_owned(), "x".repeat(2_001)] {
        let refused = r.core().send_text(view.id, bad).await.unwrap_err();
        assert!(matches!(refused, CoreError::InvalidInput(_)), "{refused:?}");
    }
    assert!(matches!(
        r.core().edit_turn(view.id, 1, "changed".to_owned()).await,
        Err(CoreError::Conflict(_))
    ));
    assert!(matches!(
        r.core().push_to_talk(view.id, true).await,
        Err(CoreError::Conflict(_))
    ));
    assert!(matches!(
        r.core().next_activity(view.id).await,
        Err(CoreError::Conflict(_))
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_message_while_a_reply_is_running_is_refused_as_busy() {
    let release = Arc::new(Semaphore::new(0));
    let r = rig(Setup {
        steps: vec![
            reply("Hello."),
            Step::Reply(
                ReplyScript::new(&["One ", "two ", "three."]).hold_after(0, release.clone()),
            ),
        ],
        ..Setup::default()
    })
    .await;
    let view = r.start(chat_request("food")).await;
    r.events.reply_stored(0).await;
    r.events
        .wait("the opening to end", |events| {
            events.iter().any(|e| {
                matches!(
                    e,
                    ServerEvent::TurnState {
                        tutor_turn_seq: Some(_),
                        ..
                    }
                )
            })
        })
        .await;
    r.core()
        .send_text(view.id, "First.".to_owned())
        .await
        .unwrap();
    r.events
        .wait("the first piece of the reply", |events| {
            events
                .iter()
                .any(|e| matches!(e, ServerEvent::TutorTextDelta { delta, .. } if delta == "One "))
        })
        .await;

    let refused = r
        .core()
        .send_text(view.id, "Second.".to_owned())
        .await
        .unwrap_err();
    assert!(matches!(refused, CoreError::Busy), "{refused:?}");

    // A page that is reloaded now reads the session, with the reply so far.
    let snapshot = r.core().snapshot();
    let active = snapshot.active_session.expect("the session");
    assert_eq!(active.turn_state, Some(TurnPhase::Replying));
    assert_eq!(active.pending_reply.as_deref(), Some("One "));
    assert!(
        active
            .recent
            .iter()
            .any(|l| l.role == LineRole::Learner && l.text == "First."),
        "{:?}",
        active.recent
    );

    release.add_permits(8);
    r.events
        .wait("the reply to end", |events| {
            events
                .iter()
                .filter(|e| {
                    matches!(
                        e,
                        ServerEvent::TurnState {
                            tutor_turn_seq: Some(_),
                            ..
                        }
                    )
                })
                .count()
                == 2
        })
        .await;
    let done = r.core().snapshot().active_session.unwrap();
    assert_eq!(done.pending_reply, None);
    assert!(
        done.recent
            .iter()
            .any(|l| l.role == LineRole::Tutor && l.text.contains("One two three.")),
        "{:?}",
        done.recent
    );
    assert_eq!(done.turns_completed, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn pause_and_resume_move_the_session_and_a_paused_chat_takes_no_message() {
    let r = rig(Setup {
        steps: vec![reply("Hello."), reply("Welcome back.")],
        ..Setup::default()
    })
    .await;
    let view = r.start(chat_request("food")).await;
    r.events
        .wait("the opening to end", |events| {
            events.iter().any(|e| {
                matches!(
                    e,
                    ServerEvent::TurnState {
                        tutor_turn_seq: Some(_),
                        ..
                    }
                )
            })
        })
        .await;

    let paused = r.core().pause_session(view.id).await.unwrap();
    assert_eq!(paused.life, SessionLife::Paused);
    assert_eq!(paused.turn_state, None);
    assert_eq!(
        r.core().snapshot().active_session.unwrap().life,
        SessionLife::Paused
    );
    assert!(matches!(
        r.core().send_text(view.id, "Anyone?".to_owned()).await,
        Err(CoreError::Conflict(_))
    ));

    let mark = r.mark().await;
    let resumed = r.core().resume_session(view.id).await.unwrap();
    assert_eq!(resumed.life, SessionLife::Active);
    let events = r.events.settled().await;
    assert_eq!(names_after(&events, mark), ["SessionState", "TurnState"]);
    r.core()
        .send_text(view.id, "I am back.".to_owned())
        .await
        .expect("a resumed chat takes a message");
    r.events.wait_for("AnalysisReady", 0).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reply_that_is_stopped_pauses_the_session_and_keeps_what_arrived() {
    let release = Arc::new(Semaphore::new(0));
    let r = rig(Setup {
        steps: vec![
            reply("Hello."),
            Step::Reply(ReplyScript::new(&["Partly ", "done."]).hold_after(0, release)),
        ],
        ..Setup::default()
    })
    .await;
    let view = r.start(chat_request("food")).await;
    r.events
        .wait("the opening to end", |events| {
            events.iter().any(|e| {
                matches!(
                    e,
                    ServerEvent::TurnState {
                        tutor_turn_seq: Some(_),
                        ..
                    }
                )
            })
        })
        .await;
    r.core()
        .send_text(view.id, "Tell me more.".to_owned())
        .await
        .unwrap();
    r.events
        .wait("the first piece", |events| {
            events.iter().any(
                |e| matches!(e, ServerEvent::TutorTextDelta { delta, .. } if delta == "Partly "),
            )
        })
        .await;
    let paused = r.core().pause_session(view.id).await.unwrap();
    assert_eq!(paused.life, SessionLife::Paused);
    let active = r.core().snapshot().active_session.unwrap();
    assert_eq!(
        active.pending_reply, None,
        "the partial reply became a line"
    );
    assert!(
        active
            .recent
            .iter()
            .any(|l| l.role == LineRole::Tutor && l.text.starts_with("Partly")),
        "{:?}",
        active.recent
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_chat_needs_a_provider_and_says_so_while_a_lesson_does_not() {
    let r = rig(Setup {
        llm: false,
        ..Setup::default()
    })
    .await;
    let refused = r
        .core()
        .start_session(chat_request("food"))
        .await
        .unwrap_err();
    assert!(
        matches!(refused, CoreError::ProviderNotConfigured),
        "{refused:?}"
    );
    assert_eq!(refused.body().error, ErrorCode::ProviderNotConfigured);
    assert!(
        r.core().snapshot().active_session.is_none(),
        "a refused start leaves no session and gives the slot back"
    );
    let lesson = r
        .core()
        .start_session(unit_request(SessionKind::Lesson, "a1-u01"))
        .await
        .expect("a lesson starts without a provider");
    assert_eq!(lesson.kind, SessionKind::Lesson);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_chat_needs_a_topic_and_a_bank_topic_needs_the_bank() {
    let r = rig(Setup::default()).await;
    let none = r
        .core()
        .start_session(request(SessionKind::TextChat))
        .await
        .unwrap_err();
    assert!(matches!(none, CoreError::InvalidInput(_)), "{none:?}");
    let bank = r
        .core()
        .start_session(app_core::api::StartSessionRequest {
            topic: Some(app_core::api::TopicChoice::Bank {
                id: "food".to_owned(),
            }),
            ..request(SessionKind::TextChat)
        })
        .await
        .unwrap_err();
    assert!(matches!(bank, CoreError::Unavailable { .. }), "{bank:?}");
    assert!(bank.body().message.contains("topic bank"));
    assert!(r.core().snapshot().active_session.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn review_and_placement_name_the_engine_that_is_missing() {
    let r = rig(Setup::default()).await;
    for (kind, word) in [
        (SessionKind::Review, "review"),
        (SessionKind::Placement, "placement"),
    ] {
        let refused = r.core().start_session(request(kind)).await.unwrap_err();
        let body = refused.body();
        assert_eq!(body.error, ErrorCode::NotAvailable);
        assert!(body.message.contains(word), "{}", body.message);
        assert!(r.core().snapshot().active_session.is_none());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_snapshot_says_what_loaded_and_lists_the_rest_as_unavailable() {
    // No audio, no speech and no manifest: sessions, activities and the free modes
    // work, speech and models do not.
    let r = rig(Setup::default()).await;
    let snapshot = r.core().snapshot();
    assert_eq!(snapshot.unavailable, [Feature::Speech, Feature::Models]);
    let by_id = |id: EngineId| snapshot.engines.iter().find(|e| e.id == id).unwrap();
    for id in [
        EngineId::AudioInput,
        EngineId::AudioOutput,
        EngineId::Vad,
        EngineId::Stt,
        EngineId::Tts,
        EngineId::Pron,
    ] {
        assert_eq!(by_id(id).state, EngineState::Unavailable, "{id:?}");
        assert!(!by_id(id).detail.is_empty());
    }

    // Without units the activities are not available either.
    let bare = rig(Setup {
        unit: false,
        ..Setup::default()
    })
    .await;
    assert_eq!(
        bare.core().snapshot().unavailable,
        [Feature::Activities, Feature::Speech, Feature::Models]
    );

    // With fake engines, speech is available.
    let voiced = rig(Setup {
        speech: Some(Vec::new()),
        ..Setup::default()
    })
    .await;
    let snapshot = voiced.core().snapshot();
    assert_eq!(snapshot.unavailable, [Feature::Models]);
    assert_eq!(
        snapshot
            .engines
            .iter()
            .find(|e| e.id == EngineId::Stt)
            .unwrap()
            .state,
        EngineState::Configured
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_client_that_connects_mid_session_gets_a_snapshot_with_the_session_and_the_event_numbering_goes_on()
 {
    let r = rig(Setup {
        steps: vec![reply("Hello there."), reply("Good.")],
        ..Setup::default()
    })
    .await;
    let view = r.start(chat_request("food")).await;
    r.events
        .wait("the opening to end", |events| {
            events.iter().any(|e| {
                matches!(
                    e,
                    ServerEvent::TurnState {
                        tutor_turn_seq: Some(_),
                        ..
                    }
                )
            })
        })
        .await;

    // What the socket does on connect: subscribe, then take the snapshot.
    let mut rx = r.core().events().subscribe();
    let ServerEvent::Snapshot { seq, state } = r.core().snapshot_event() else {
        panic!("a snapshot");
    };
    assert_eq!(seq, r.core().events().current_seq());
    let active = state.active_session.expect("the running session");
    assert_eq!(active.id, view.id);
    assert_eq!(active.kind, SessionKind::TextChat);
    assert!(
        active
            .recent
            .iter()
            .any(|l| l.role == LineRole::Tutor && l.text == "Hello there."),
        "the page can rebuild the conversation: {:?}",
        active.recent
    );

    r.core().send_text(view.id, "Hi.".to_owned()).await.unwrap();
    let first = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("an event")
        .expect("open");
    assert_eq!(first.seq(), seq + 1, "the stream goes on from the snapshot");
}
