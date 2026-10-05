#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! The voice loop over fakes: a scripted language model, fake speech engines
//! and a fake audio backend. No hardware and no network.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use app_core::voice::testing::{FailKind, ReplyScript, Step};
use app_core::voice::{StopCause, TurnOutcome, VoiceError, VoiceEvent};
use common::voice::{Audio, Options, Rig};
use llm_client::{FinishReason, Role};
use tokio::sync::Semaphore;
use tutor_engine::{EngineFault, FALLBACK_LINE, Phase, TurnState};

fn active(turn: TurnState) -> Phase {
    Phase::Active { turn }
}

fn reply(deltas: &[&str]) -> Step {
    Step::Reply(ReplyScript::new(deltas))
}

fn index_of(events: &[VoiceEvent], wanted: impl Fn(&VoiceEvent) -> bool) -> usize {
    events
        .iter()
        .position(wanted)
        .unwrap_or_else(|| panic!("event not found in {events:#?}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_scripted_conversation_runs_turn_by_turn_and_speaks_sentence_by_sentence() {
    let steps = vec![
        reply(&["Hello there. ", "What is your name?"]),
        reply(&["Nice to meet you, ", "Dewi. ", "Where are you from?"]),
        reply(&["Great. ", "See you!"]),
    ];
    let mut rig = Rig::start(
        steps,
        Options {
            transcripts: vec!["My name is Dewi.", "I am from Bandung."],
            ..Options::default()
        },
    )
    .await;
    let backend = rig.audio.as_ref().expect("audio").backend.inner.clone();

    rig.handle.open().await.unwrap();
    rig.log.turn_ended(1).await;
    rig.say().await;
    rig.log.turn_ended(2).await;
    rig.say().await;
    rig.log.turn_ended(3).await;

    // Turn order, as the tutor engine's phases.
    use TurnState::{Listening, Speaking, Thinking, Transcribing};
    assert_eq!(
        rig.log.phases(),
        [
            active(Thinking),
            active(Speaking),
            active(Listening),
            active(Transcribing),
            active(Thinking),
            active(Speaking),
            active(Listening),
            active(Transcribing),
            active(Thinking),
            active(Speaking),
            active(Listening),
        ]
    );
    let heard: Vec<String> = rig
        .log
        .all()
        .into_iter()
        .filter_map(|e| match e {
            VoiceEvent::Heard { text, .. } => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(heard, ["My name is Dewi.", "I am from Bandung."]);

    // The chunker feeds TTS one sentence at a time, in order.
    let expected = [
        "Hello there.",
        "What is your name?",
        "Nice to meet you, Dewi.",
        "Where are you from?",
        "Great.",
        "See you!",
    ];
    assert_eq!(rig.log.sentences(), expected);
    let spoken = rig.tts_log.as_ref().unwrap().spoken.lock().unwrap().clone();
    assert_eq!(spoken, expected);

    // The model got the T1 prompt and the conversation so far.
    let requests = rig.llm.requests();
    assert_eq!(requests.len(), 3);
    assert!(
        requests[0]
            .system
            .contains("talking with one learner by voice")
    );
    assert!(requests[0].system.contains("Sam, a friendly new classmate"));
    assert_eq!(requests[0].max_tokens, 80);
    assert_eq!(requests[0].temperature, Some(0.7));
    let roles: Vec<Role> = requests[1].messages.iter().map(|m| m.role).collect();
    assert_eq!(roles, [Role::User, Role::Assistant, Role::User]);
    assert!(
        requests[1].messages[2]
            .content
            .contains("<learner_said>\nMy name is Dewi.\n</learner_said>")
    );
    assert_eq!(
        requests[1].messages[1].content,
        "Hello there. What is your name?"
    );

    // The engines that loaded identify themselves, for the result files.
    let engines = rig.handle.engines();
    assert_eq!(engines.stt.map(|i| i.id).as_deref(), Some("fake-stt"));
    assert_eq!(engines.tts.map(|i| i.id).as_deref(), Some("fake-tts"));

    let summary = rig.finish().await;
    assert_eq!(summary.turns_completed, 3);
    assert_eq!(summary.latencies.len(), 3);
    assert_eq!(summary.stats.frames_dropped, 0);
    assert_eq!(summary.stats.inbox_dropped, 0);
    // Nothing is left running: every stream and thread of the backend is gone.
    assert_eq!(backend.live_streams(), 0);
    assert_eq!(backend.live_threads(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_first_sentence_is_audible_before_the_language_model_stream_ends() {
    let permit = Arc::new(Semaphore::new(0));
    let steps = vec![Step::Reply(
        ReplyScript::new(&["Hello there. ", "How are you today? ", "Tell me."])
            .hold_after(0, permit.clone()),
    )];
    let mut rig = Rig::start(steps, Options::default()).await;
    rig.handle.open().await.unwrap();

    // The first tutor sample plays while the stream is held open.
    let events = rig
        .log
        .wait("the first sample", |e| {
            e.iter().any(|e| matches!(e, VoiceEvent::Latency(_)))
        })
        .await;
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, VoiceEvent::TurnEnded { .. })),
        "the turn cannot be over while the stream is held"
    );
    assert_eq!(rig.log.sentences(), ["Hello there."]);
    assert_eq!(
        rig.tts_log.as_ref().unwrap().spoken.lock().unwrap().clone(),
        ["Hello there."]
    );
    assert_eq!(rig.handle.phase(), active(TurnState::Speaking));

    permit.add_permits(1);
    rig.log.turn_ended(1).await;
    assert_eq!(
        rig.tts_log.as_ref().unwrap().spoken.lock().unwrap().clone(),
        ["Hello there.", "How are you today?", "Tell me."]
    );
    rig.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stop_cancels_the_stream_and_the_tts_turn_and_flushes_playback() {
    let never = Arc::new(Semaphore::new(0));
    // Three sentences arrive at once and the synthesiser takes 150 ms for each,
    // so two are still waiting when the learner stops the tutor.
    let steps = vec![Step::Reply(
        ReplyScript::new(&["One is here. Two is here. Three is here. ", "never sent"])
            .hold_after(0, never),
    )];
    let mut rig = Rig::start(
        steps,
        Options {
            audio: Audio::Full,
            tts_slow: Duration::from_millis(150),
            ..Options::default()
        },
    )
    .await;
    rig.handle.open().await.unwrap();
    rig.log
        .wait("the tutor to speak", |e| {
            e.iter().any(|e| matches!(e, VoiceEvent::Latency(_)))
        })
        .await;
    assert_eq!(rig.handle.phase(), active(TurnState::Speaking));

    let asked = Instant::now();
    rig.handle.stop_speaking().await.unwrap();
    rig.log
        .wait("the stop", |e| {
            e.iter()
                .any(|e| matches!(e, VoiceEvent::Stopped(StopCause::Command)))
        })
        .await;
    // The model stream sees its token cancelled.
    while rig.llm.cancellations() == 0 {
        assert!(
            asked.elapsed() < Duration::from_millis(500),
            "stream not cancelled"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    let reaction = asked.elapsed();
    assert!(reaction < Duration::from_millis(500), "{reaction:?}");
    // The stream was dropped before its last event: the connection is closed.
    assert_eq!(rig.llm.closed_early(), 1);
    assert_eq!(rig.handle.phase(), active(TurnState::Listening));

    // Playback is flushed and stays silent: nothing more is played.
    let stats = rig.handle.stats();
    let audio = stats.audio.expect("a session");
    assert_eq!(audio.playback.samples_queued, 0);
    tokio::time::sleep(Duration::from_millis(400)).await;
    let later = rig.handle.stats();
    assert_eq!(
        later.audio.unwrap().playback.samples_played,
        audio.playback.samples_played,
        "nothing plays after the flush"
    );
    // The sentences that had not been synthesised ended as cancelled.
    assert!(
        later.tts.unwrap().cancelled >= 2,
        "{:?}",
        rig.handle.stats().tts
    );
    assert!(
        rig.tts_log.as_ref().unwrap().spoken.lock().unwrap().len() <= 1,
        "only the first sentence reached the engine"
    );
    // The turn that was stopped did not count as a finished one.
    assert_eq!(rig.handle.turns_completed(), 0);
    rig.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn speaking_while_the_tutor_thinks_cancels_the_turn() {
    let mut rig = Rig::start(
        vec![Step::Hang],
        Options {
            transcripts: vec!["Hello."],
            ..Options::default()
        },
    )
    .await;
    rig.handle.open().await.unwrap();
    rig.log
        .wait("thinking", |_| {
            rig.handle.phase() == active(TurnState::Thinking)
        })
        .await;
    rig.say().await;
    rig.log
        .wait("the turn to be cancelled by speech", |e| {
            e.iter()
                .any(|e| matches!(e, VoiceEvent::Stopped(StopCause::Speech)))
        })
        .await;
    assert_eq!(rig.llm.cancellations(), 1);
    rig.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_gate_keeps_the_microphone_closed_while_the_tutor_speaks() {
    let steps = vec![reply(&[
        "This reply is long enough to play for about two seconds.",
    ])];
    let mut rig = Rig::start(
        steps,
        Options {
            audio: Audio::Full,
            ..Options::default()
        },
    )
    .await;
    let backend = rig.audio.as_ref().unwrap().backend.inner.clone();

    rig.handle.open().await.unwrap();
    rig.log
        .wait("speaking", |e| {
            e.iter().any(|e| matches!(e, VoiceEvent::Latency(_)))
        })
        .await;
    // The learner's microphone now hears loud sound, as if the tutor's own
    // voice came back through the speakers.
    backend.set_input_tone(Some(220.0));
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let dropped = rig.handle.stats().audio.unwrap().gate.dropped;
        if dropped >= 20 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "no frame was dropped by the gate"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        !rig.log
            .all()
            .iter()
            .any(|e| matches!(e, VoiceEvent::SpeechStarted)),
        "the VAD must not hear anything while the tutor speaks"
    );

    // When playback is over and the 150 ms are up, the same sound is heard.
    let events = rig
        .log
        .wait("the gate to open again", |e| {
            e.iter().any(|e| matches!(e, VoiceEvent::SpeechStarted))
        })
        .await;
    let ended = index_of(&events, |e| matches!(e, VoiceEvent::TurnEnded { .. }));
    let heard = index_of(&events, |e| matches!(e, VoiceEvent::SpeechStarted));
    assert!(ended < heard, "speech was heard only after the reply ended");
    let stats = rig.handle.stats().audio.unwrap();
    assert!(stats.gate.dropped >= 20);
    assert!(stats.gate.to_vad > 0);
    rig.finish().await;
    backend.set_input_tone(None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_call_is_retried_once_and_the_reply_is_spoken() {
    let steps = vec![Step::Fail(FailKind::Unavailable), reply(&["Welcome back."])];
    let mut rig = Rig::start(steps, Options::default()).await;
    rig.handle.open().await.unwrap();
    let events = rig.log.turn_ended(1).await;
    assert_eq!(rig.llm.calls(), 2, "one failure, one retry");
    assert!(events.iter().any(|e| matches!(
        e,
        VoiceEvent::TurnEnded {
            outcome: TurnOutcome::Replied,
            ..
        }
    )));
    assert_eq!(rig.log.sentences(), ["Welcome back."]);
    rig.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn two_failures_end_in_provider_unavailable_with_a_clear_message_and_resume_recovers() {
    let steps = vec![
        Step::Fail(FailKind::Unreachable),
        Step::Fail(FailKind::Unreachable),
        reply(&["I am back."]),
    ];
    let mut rig = Rig::start(steps, Options::default()).await;
    rig.handle.open().await.unwrap();
    let events = rig.log.turn_ended(1).await;
    assert_eq!(
        rig.llm.calls(),
        2,
        "the turn is tried once more, then given up"
    );
    let message = events
        .iter()
        .find_map(|e| match e {
            VoiceEvent::ProviderUnavailable { message } => Some(message.clone()),
            _ => None,
        })
        .expect("a provider event");
    assert!(message.contains("language model provider"), "{message}");
    assert!(message.contains("resume"), "{message}");
    assert_eq!(rig.handle.phase(), Phase::ProviderUnavailable);
    assert!(events.iter().any(|e| matches!(
        e,
        VoiceEvent::TurnEnded {
            outcome: TurnOutcome::ProviderUnavailable,
            ..
        }
    )));
    assert!(
        rig.tts_log
            .as_ref()
            .unwrap()
            .spoken
            .lock()
            .unwrap()
            .is_empty(),
        "nothing was spoken"
    );

    rig.handle.resume().await.unwrap();
    rig.handle.send_text("Hello again").await.unwrap();
    rig.log.turn_ended(2).await;
    assert_eq!(rig.log.sentences(), ["I am back."]);
    rig.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_provider_that_never_answers_is_given_up_within_the_budget() {
    // Two attempts of 200 ms each.
    let mut rig = Rig::start(
        vec![Step::Hang, Step::Hang],
        Options {
            provider_timeout: Duration::from_millis(400),
            ..Options::default()
        },
    )
    .await;
    let started = Instant::now();
    rig.handle.open().await.unwrap();
    rig.log
        .wait("ProviderUnavailable", |e| {
            e.iter()
                .any(|e| matches!(e, VoiceEvent::ProviderUnavailable { .. }))
        })
        .await;
    let waited = started.elapsed();
    assert!(waited >= Duration::from_millis(390), "{waited:?}");
    assert!(waited < Duration::from_millis(1_200), "{waited:?}");
    assert_eq!(rig.llm.calls(), 2);
    rig.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_reply_gets_the_authored_line_once_and_then_the_provider_is_unavailable() {
    let steps = vec![
        Step::Empty(FinishReason::Stop),
        Step::Empty(FinishReason::Refusal),
    ];
    let mut rig = Rig::start(steps, Options::default()).await;
    rig.handle.open().await.unwrap();
    let first = rig.log.turn_ended(1).await;
    assert!(first.iter().any(|e| matches!(
        e,
        VoiceEvent::TurnEnded {
            outcome: TurnOutcome::Fallback,
            ..
        }
    )));
    assert_eq!(rig.log.sentences(), [FALLBACK_LINE]);

    rig.handle.send_text("Hello?").await.unwrap();
    let second = rig.log.turn_ended(2).await;
    assert!(second.iter().any(|e| matches!(
        e,
        VoiceEvent::TurnEnded {
            outcome: TurnOutcome::ProviderUnavailable,
            ..
        }
    )));
    assert_eq!(rig.handle.phase(), Phase::ProviderUnavailable);
    rig.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn nothing_heard_gets_the_authored_line_without_asking_the_model() {
    let mut rig = Rig::start(
        Vec::new(),
        Options {
            transcripts: vec![""],
            ..Options::default()
        },
    )
    .await;
    rig.say().await;
    rig.log.turn_ended(1).await;
    assert_eq!(rig.llm.calls(), 0);
    assert_eq!(rig.log.sentences(), [FALLBACK_LINE]);
    rig.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_synthesis_failure_ends_the_turn_as_an_engine_error_and_the_next_turn_works() {
    let steps = vec![
        reply(&["This one is fine. ", "This one is boom. "]),
        reply(&["Now it works."]),
    ];
    let mut rig = Rig::start(
        steps,
        Options {
            tts_fail_on: Some("boom"),
            ..Options::default()
        },
    )
    .await;
    rig.handle.open().await.unwrap();
    let events = rig.log.turn_ended(1).await;
    assert!(events.iter().any(|e| matches!(
        e,
        VoiceEvent::EngineError {
            fault: EngineFault::SpeechSynthesis,
            ..
        }
    )));
    assert!(events.iter().any(|e| matches!(
        e,
        VoiceEvent::TurnEnded {
            outcome: TurnOutcome::EngineError,
            ..
        }
    )));
    // The session recovered to listening and the next turn is spoken.
    assert_eq!(rig.handle.phase(), active(TurnState::Listening));
    rig.handle.send_text("Try again").await.unwrap();
    rig.log.turn_ended(2).await;
    assert!(rig.log.sentences().contains(&"Now it works.".to_owned()));
    rig.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_recogniser_that_cannot_load_stops_the_start_and_leaves_nothing_running() {
    let error = Rig::try_start(
        Vec::new(),
        Options {
            stt_fails: true,
            ..Options::default()
        },
    )
    .await
    .err()
    .expect("start fails");
    assert!(
        matches!(&error, VoiceError::EngineUnavailable { engine, .. } if *engine == "speech recognition"),
        "{error:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn typing_works_without_a_microphone_and_the_latency_has_only_the_model_part() {
    let steps = vec![reply(&["Hello. ", "Nice to meet you."])];
    let mut rig = Rig::start(
        steps,
        Options {
            audio: Audio::None,
            listen: false,
            tts: false,
            ..Options::default()
        },
    )
    .await;
    let fed = rig.handle.feed_audio(&[0.0; 512]).await;
    assert!(matches!(fed, Err(VoiceError::Missing(_))));
    rig.handle.send_text("Hello").await.unwrap();
    rig.log.turn_ended(1).await;
    assert_eq!(rig.log.sentences(), ["Hello.", "Nice to meet you."]);
    let summary = rig.finish().await;
    assert_eq!(summary.latencies.len(), 1);
    let parts = summary.latencies[0].parts;
    assert!(parts.llm_first_sentence.is_some());
    assert!(parts.endpointing_wait.is_none() && parts.output_start.is_none());
    assert!(!summary.latencies[0].is_complete());
    assert_eq!(summary.latency.complete_turns, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn latency_parts_are_read_from_the_injected_clock_and_add_up() {
    let steps = vec![Step::Reply(
        ReplyScript::new(&["Hello there. "]).after(Duration::from_millis(900)),
    )];
    let mut rig = Rig::start(
        steps,
        Options {
            transcripts: vec!["Hi."],
            stt_finalise: Duration::from_millis(700),
            tts_latency: Duration::from_millis(300),
            ..Options::default()
        },
    )
    .await;
    let backend = rig.audio.as_ref().unwrap().backend.clone();
    // The output callback plays nothing until the test lets it, so the clock can
    // be moved between "audio queued" and "first sample played".
    backend.hold(true);
    rig.say().await;
    let deadline = Instant::now() + Duration::from_secs(5);
    // Audio is in the queue once the sink has stamped the clock and handed it over.
    while rig.handle.stats().playback_queued.is_zero() {
        assert!(
            Instant::now() < deadline,
            "the first sentence was not queued"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    rig.clock.advance(Duration::from_millis(50));
    backend.hold(false);

    let events = rig.log.turn_ended(1).await;
    let latency = events
        .iter()
        .find_map(|e| match e {
            VoiceEvent::Latency(l) => Some(*l),
            _ => None,
        })
        .expect("a latency event");
    let ms = |d: Option<Duration>| d.map(|d| d.as_millis());
    assert_eq!(
        ms(latency.parts.endpointing_wait),
        Some(608),
        "19 frames of 32 ms"
    );
    assert_eq!(ms(latency.parts.stt_finalise), Some(700));
    assert_eq!(ms(latency.parts.llm_first_sentence), Some(900));
    assert_eq!(ms(latency.parts.tts_first_sentence), Some(300));
    assert_eq!(ms(latency.parts.output_start), Some(50));
    assert!(latency.is_complete());
    assert_eq!(
        latency.sum(),
        Duration::from_millis(608 + 700 + 900 + 300 + 50)
    );

    let summary = rig.finish().await;
    assert_eq!(summary.latencies, [latency]);
    assert_eq!(summary.latency.complete_turns, 1);
    assert_eq!(summary.latency.sum.p50_ms, Some(2_558.0));
    assert_eq!(summary.latency.sum.p95_ms, Some(2_558.0));
}

#[tokio::test(flavor = "multi_thread")]
async fn losing_the_microphone_is_reported_and_the_session_recovers_on_another_device() {
    let mut rig = Rig::start(
        Vec::new(),
        Options {
            audio: Audio::Full,
            ..Options::default()
        },
    )
    .await;
    let audio = rig.audio.as_ref().unwrap();
    let backend = audio.backend.inner.clone();
    // A second microphone is ready when the first one disappears.
    backend.add_device(
        "Second microphone",
        audio_io::Direction::Input,
        audio_io::StreamFormat::new(16_000, 1),
        false,
    );
    backend.unplug(&audio.microphone);
    let events = rig
        .log
        .wait("the loss and the recovery", |e| {
            e.iter().any(|e| {
                matches!(
                    e,
                    VoiceEvent::EngineError {
                        fault: EngineFault::Microphone,
                        ..
                    }
                )
            }) && e
                .iter()
                .any(|e| matches!(e, VoiceEvent::State(p) if *p == active(TurnState::Listening)))
        })
        .await;
    assert!(events.iter().any(|e| matches!(
        e,
        VoiceEvent::State(Phase::EngineError {
            fault: EngineFault::Microphone
        })
    )));
    assert_eq!(rig.handle.phase(), active(TurnState::Listening));
    rig.finish().await;
}
