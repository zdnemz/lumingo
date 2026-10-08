#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! A voice conversation over the session manager, with fake devices and fake
//! speech engines: the microphone is a tone the test switches on and off.

mod common;

use std::time::Duration;

use app_core::api::EditEffect;
use app_core::api::{
    EngineId, EngineState, ErrorCode, FeedbackView, LineRole, ServerEvent, SessionChannel,
    SessionKind, SessionLife, SessionStatus, StartSessionRequest, StopRequest, TopicChoice,
    TurnPhase,
};
use app_core::error::CoreError;
use app_core::voice::testing::{ReplyScript, Step};
use common::sessions::{SessionRig, Setup, request, rig};

fn reply(text: &str) -> Step {
    Step::Reply(ReplyScript::new(&[text]))
}

fn conversation(topic: &str) -> StartSessionRequest {
    StartSessionRequest {
        topic: Some(TopicChoice::Typed {
            text: topic.to_owned(),
        }),
        ..request(SessionKind::Conversation)
    }
}

fn is_listening(events: &[ServerEvent], after: usize) -> bool {
    events.iter().skip(after).any(|e| {
        matches!(
            e,
            ServerEvent::TurnState {
                state: TurnPhase::Listening,
                ..
            }
        )
    })
}

/// One utterance: a tone for 800 ms, then silence until the end is heard.
async fn speak_one_utterance(r: &SessionRig) {
    let backend = r.audio.as_ref().unwrap().backend.inner.clone();
    backend.set_input_tone(Some(440.0));
    tokio::time::sleep(Duration::from_millis(800)).await;
    backend.set_input_tone(None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_spoken_turn_sends_its_events_in_order_and_the_stored_line_and_analysis_after_the_reply()
{
    let r = rig(Setup {
        steps: vec![
            reply("Hello! What is your name?"),
            reply("Nice to meet you, Dewi."),
        ],
        speech: Some(vec!["my name is Dewi"]),
        ..Setup::default()
    })
    .await;
    let view = r.start(conversation("introductions")).await;
    assert_eq!(view.kind, SessionKind::Conversation);
    assert_eq!(view.channel, SessionChannel::Voice);
    assert!(view.speaking);

    // The tutor speaks first, then listens.
    let events = r
        .events
        .wait("the opening to end", |events| {
            events
                .iter()
                .any(|e| matches!(e, ServerEvent::TutorSentenceSpoken { .. }))
                && is_listening(events, 1)
        })
        .await;
    // Engines report while they load, before the session exists; the first event
    // of the session itself is its state.
    assert!(
        matches!(
            events
                .iter()
                .find(|e| !matches!(e, ServerEvent::EngineStatus { .. })),
            Some(ServerEvent::SessionState {
                life: SessionLife::Active,
                kind: SessionKind::Conversation,
                ..
            })
        ),
        "the first event is the session becoming active: {:?}",
        r.events.names()
    );
    let said = r
        .speech
        .as_ref()
        .unwrap()
        .tts_log
        .spoken
        .lock()
        .unwrap()
        .clone();
    assert_eq!(said, ["Hello!", "What is your name?"]);

    let mark = r.mark().await;
    speak_one_utterance(&r).await;
    let events = r.events.wait_for("AnalysisReady", mark).await;
    let after: Vec<&ServerEvent> = events.iter().skip(mark).collect();
    let index_of = |pick: &dyn Fn(&ServerEvent) -> bool| after.iter().position(|e| pick(e));

    let heard = index_of(&|e| {
        matches!(
            e,
            ServerEvent::TranscriptFinal {
                turn_seq: None,
                source: SessionChannel::Voice,
                ..
            }
        )
    })
    .expect("the line when it was heard");
    let thinking = index_of(&|e| {
        matches!(
            e,
            ServerEvent::TurnState {
                state: TurnPhase::Thinking,
                ..
            }
        )
    })
    .expect("thinking");
    let sentence = index_of(&|e| matches!(e, ServerEvent::TutorSentenceSpoken { .. }))
        .expect("the reply is spoken");
    let latency = index_of(&|e| matches!(e, ServerEvent::LatencyReport { .. }))
        .expect("the latency of the turn");
    let stored = index_of(&|e| {
        matches!(
            e,
            ServerEvent::TranscriptFinal {
                turn_seq: Some(_),
                ..
            }
        )
    })
    .expect("the line when it was stored");
    let analysis = index_of(&|e| matches!(e, ServerEvent::AnalysisReady { .. })).unwrap();
    let sequence: Vec<&str> = after.iter().map(|e| e.name()).collect();
    assert!(heard < thinking, "heard, then thinking: {sequence:?}");
    assert!(thinking < sentence, "thinking, then the reply");
    assert!(sentence < stored, "the line is stored after the reply");
    assert!(stored < analysis, "the analysis is of a stored line");
    assert!(latency > sentence || latency < analysis);

    let ServerEvent::TranscriptFinal { text, turn, .. } = after[heard] else {
        panic!("a line");
    };
    assert_eq!(text, "my name is Dewi");
    let ServerEvent::TranscriptFinal {
        turn: stored_turn,
        turn_seq: Some(seq),
        ..
    } = after[stored]
    else {
        panic!("a stored line");
    };
    assert_eq!(
        stored_turn, turn,
        "both announcements carry the turn number"
    );
    let ServerEvent::AnalysisReady {
        analysis: view_of, ..
    } = after[analysis]
    else {
        panic!("an analysis");
    };
    assert_eq!(view_of.turn_seq, *seq);

    // The analysis never comes before the end of the reply it belongs to.
    let reply_ended = after
        .iter()
        .rposition(|e| matches!(e, ServerEvent::TutorSentenceSpoken { turn: t, .. } if t == turn))
        .unwrap();
    assert!(reply_ended < analysis);

    // The snapshot a reloaded page reads holds the conversation.
    let active = r.core().snapshot().active_session.unwrap();
    assert!(
        active
            .recent
            .iter()
            .any(|l| l.role == LineRole::Learner && l.seq == Some(*seq))
    );
    assert!(active.recent.iter().any(|l| l.role == LineRole::Tutor));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_microphone_level_reaches_the_stream_and_stays_within_ten_a_second() {
    let r = rig(Setup {
        steps: vec![reply("Hello.")],
        speech: Some(vec!["hello"]),
        ..Setup::default()
    })
    .await;
    r.start(conversation("food")).await;
    r.events
        .wait("the opening", |events| is_listening(events, 1))
        .await;
    let mark = r.mark().await;
    let started = std::time::Instant::now();
    r.audio
        .as_ref()
        .unwrap()
        .backend
        .inner
        .set_input_tone(Some(440.0));
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    r.audio.as_ref().unwrap().backend.inner.set_input_tone(None);
    let took = started.elapsed();
    let levels: Vec<f32> = r
        .events
        .all()
        .iter()
        .skip(mark)
        .filter_map(|e| match e {
            ServerEvent::MicLevel { level, .. } => Some(*level),
            _ => None,
        })
        .collect();
    assert!(levels.iter().any(|l| *l > 0.2), "{levels:?}");
    assert!(
        levels.len() as f64 <= took.as_secs_f64() * 10.0 + 1.0,
        "{} levels in {took:?}",
        levels.len()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_corrected_transcript_is_stored_with_the_original_and_marked_edited() {
    let r = rig(Setup {
        steps: vec![
            reply("Hello."),
            reply("Nice to meet you."),
            reply("Good to know."),
        ],
        speech: Some(vec!["my name is dewy"]),
        ..Setup::default()
    })
    .await;
    let view = r.start(conversation("introductions")).await;
    r.events
        .wait("the opening", |events| is_listening(events, 1))
        .await;
    speak_one_utterance(&r).await;
    let events = r.events.wait_for("AnalysisReady", 0).await;
    let (turn, seq) = events
        .iter()
        .find_map(|e| match e {
            ServerEvent::TranscriptFinal {
                turn,
                turn_seq: Some(seq),
                ..
            } => Some((*turn, *seq)),
            _ => None,
        })
        .unwrap();

    let edited = r
        .core()
        .edit_turn(view.id, turn, "My name is Dewi.".to_owned())
        .await
        .unwrap();
    assert_eq!(edited.turn, turn);
    assert!(
        matches!(
            edited.effect,
            EditEffect::StoredOnly | EditEffect::AnalysedFromEdit
        ),
        "{:?}",
        edited.effect
    );
    let line = r
        .core()
        .snapshot()
        .active_session
        .unwrap()
        .recent
        .into_iter()
        .find(|l| l.role == LineRole::Learner && l.turn == turn)
        .unwrap();
    assert_eq!(line.text, "My name is Dewi.");
    assert!(line.edited);
    assert_eq!(line.seq, Some(seq), "the stored position is kept");

    let stored = r
        .core()
        .database()
        .turns()
        .list(view.id)
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.seq == seq)
        .unwrap();
    assert_eq!(stored.text, "My name is Dewi.");
    assert_eq!(stored.stt_text.as_deref(), Some("my name is dewy"));
    assert!(stored.edited_by_learner);

    // A turn that does not exist, and a typed turn, cannot be corrected.
    assert!(matches!(
        r.core().edit_turn(view.id, 99, "x".to_owned()).await,
        Err(CoreError::NotFound { what: "turn" })
    ));
    r.core()
        .send_text(view.id, "I typed this.".to_owned())
        .await
        .unwrap();
    r.events
        .wait("the typed turn to be stored", |events| {
            events
                .iter()
                .filter(|e| {
                    matches!(
                        e,
                        ServerEvent::TranscriptFinal {
                            turn_seq: Some(_),
                            ..
                        }
                    )
                })
                .count()
                >= 2
        })
        .await;
    let typed_turn = r.core().snapshot().active_session.unwrap().turn;
    assert!(matches!(
        r.core()
            .edit_turn(view.id, typed_turn, "changed".to_owned())
            .await,
        Err(CoreError::InvalidInput(_))
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn pause_resume_push_to_talk_and_stop_speaking_are_taken_and_stop_ends_with_a_summary() {
    let r = rig(Setup {
        steps: vec![reply("Hello there. How are you today?")],
        speech: Some(vec!["fine"]),
        ..Setup::default()
    })
    .await;
    let view = r.start(conversation("feelings")).await;
    r.events
        .wait("the first sentence", |events| {
            events
                .iter()
                .any(|e| matches!(e, ServerEvent::TutorSentenceSpoken { .. }))
        })
        .await;
    assert!(r.core().stop_speaking().await.unwrap().ok);
    r.events
        .wait("listening", |events| is_listening(events, 1))
        .await;

    assert!(r.core().push_to_talk(view.id, true).await.unwrap().ok);
    assert!(r.core().push_to_talk(view.id, false).await.unwrap().ok);

    let paused = r.core().pause_session(view.id).await.unwrap();
    assert_eq!(paused.life, SessionLife::Paused);
    r.events
        .wait("the pause", |events| {
            events.iter().any(|e| {
                matches!(
                    e,
                    ServerEvent::SessionState {
                        life: SessionLife::Paused,
                        ..
                    }
                )
            })
        })
        .await;
    let mark = r.mark().await;
    r.core().resume_session(view.id).await.unwrap();
    r.events
        .wait("the resume", |events| {
            events.iter().skip(mark).any(|e| {
                matches!(
                    e,
                    ServerEvent::TurnState {
                        state: TurnPhase::Listening,
                        ..
                    }
                )
            })
        })
        .await;

    let ended = r
        .core()
        .stop_session(view.id, StopRequest::default())
        .await
        .unwrap();
    assert_eq!(ended.status, SessionStatus::Completed);
    assert!(matches!(
        ended.feedback,
        Some(FeedbackView::Conversation { .. })
    ));
    let settled = r.events.settled().await;
    let last: Vec<&'static str> = settled
        .iter()
        .rev()
        .take(2)
        .map(ServerEvent::name)
        .collect();
    assert_eq!(last, ["SessionState", "FeedbackReady"]);
    assert!(r.core().snapshot().active_session.is_none());
    let stored = r
        .core()
        .database()
        .sessions()
        .get(view.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, storage::SessionStatus::Completed);

    // The devices are free again: a standalone speech output can start.
    let devices = r.core().audio_devices().await.unwrap();
    assert_eq!(devices.inputs.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn while_a_conversation_runs_the_audio_test_is_refused_and_a_second_session_cannot_start() {
    let r = rig(Setup {
        steps: vec![reply("Hello.")],
        speech: Some(Vec::new()),
        ..Setup::default()
    })
    .await;
    let view = r.start(conversation("food")).await;
    let test = r
        .core()
        .audio_test(app_core::api::AudioTestRequest {
            duration_ms: 500,
            play_back: false,
            input_id: None,
            output_id: None,
            remember: false,
        })
        .await
        .unwrap_err();
    assert!(matches!(test, CoreError::Conflict(_)), "{test:?}");
    let second = r
        .core()
        .start_session(conversation("music"))
        .await
        .unwrap_err();
    assert!(matches!(second, CoreError::Conflict(_)), "{second:?}");
    r.core()
        .stop_session(view.id, StopRequest { cancel: true })
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_conversation_without_audio_or_speech_is_not_available_and_names_what_is_missing() {
    let r = rig(Setup {
        steps: vec![reply("Hello.")],
        ..Setup::default()
    })
    .await;
    let refused = r
        .core()
        .start_session(conversation("food"))
        .await
        .unwrap_err();
    let body = refused.body();
    assert_eq!(body.error, ErrorCode::NotAvailable);
    assert!(
        body.message.contains("microphone and speakers"),
        "{}",
        body.message
    );
    assert!(r.core().snapshot().active_session.is_none());
    // Text chat still works in the same program.
    let chat = r
        .core()
        .start_session(common::sessions::chat_request("food"))
        .await
        .unwrap();
    assert_eq!(chat.kind, SessionKind::TextChat);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_recogniser_that_fails_to_load_is_reported_as_an_engine_status_and_frees_the_slot() {
    let r = rig(Setup {
        steps: vec![reply("Hello.")],
        speech: Some(Vec::new()),
        failing_stt: true,
        ..Setup::default()
    })
    .await;
    let refused = r
        .core()
        .start_session(conversation("food"))
        .await
        .unwrap_err();
    let body = refused.body();
    assert_eq!(body.error, ErrorCode::NotAvailable);
    assert_eq!(body.feature, Some(app_core::api::Feature::Speech));

    r.events
        .wait("the engine status", |events| {
            events.iter().any(|e| {
                matches!(
                    e,
                    ServerEvent::EngineStatus { engine, .. }
                        if engine.id == EngineId::Stt && engine.state == EngineState::Failed
                )
            })
        })
        .await;
    let stt = r
        .core()
        .snapshot()
        .engines
        .into_iter()
        .find(|e| e.id == EngineId::Stt)
        .unwrap();
    assert_eq!(stt.state, EngineState::Failed);
    assert!(r.core().snapshot().active_session.is_none());
    r.core()
        .start_session(common::sessions::chat_request("food"))
        .await
        .expect("the slot is free");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_conversation_can_show_its_replies_instead_of_speaking_them() {
    let r = rig(Setup {
        steps: vec![reply("Hello. How are you?")],
        speech: Some(Vec::new()),
        ..Setup::default()
    })
    .await;
    let view = r
        .start(StartSessionRequest {
            speak: Some(false),
            ..conversation("food")
        })
        .await;
    assert!(!view.speaking);
    r.events
        .wait("the reply", |events| {
            events
                .iter()
                .any(|e| matches!(e, ServerEvent::TutorSentenceSpoken { .. }))
        })
        .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        r.speech
            .as_ref()
            .unwrap()
            .tts_log
            .spoken
            .lock()
            .unwrap()
            .is_empty(),
        "nothing was synthesised"
    );
    // A speak request would collide with the conversation's devices.
    let refused = r
        .core()
        .speak(app_core::api::SpeakRequest::Turn {
            session_id: view.id,
            turn_seq: 1,
        })
        .await
        .unwrap_err();
    assert!(matches!(refused, CoreError::Conflict(_)), "{refused:?}");
}
