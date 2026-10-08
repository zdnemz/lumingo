//! Tests of the pieces of the voice loop that need no threads.

use std::sync::Arc;
use std::time::Duration;

use audio_io::Route;
use llm_client::{ChatMessage, Role};
use speech::{EndpointConfig, UtteranceSegmenter};
use tutor_engine::{EngineFault, Event, Phase, TurnState};

use super::engine::build_messages;
use super::event::{EventSink, VoiceEvent};
use super::listen::{FrameMsg, ListenState};
use super::msg::{Counters, ListenEvent};
use super::phase::PhaseCell;
use super::testing::{EnergyVad, ManualClock, silence_audio, speech_audio};

fn frames(samples: &[f32]) -> Vec<Vec<f32>> {
    // A short last frame is padded with silence, as the live path does.
    samples
        .chunks(512)
        .map(|chunk| {
            let mut frame = chunk.to_vec();
            frame.resize(512, 0.0);
            frame
        })
        .collect()
}

fn listener(clock: &Arc<ManualClock>) -> ListenState {
    let config = EndpointConfig::new();
    ListenState::new(
        Box::new(EnergyVad::new(Some(clock.clone()), config.end_silence_ms)),
        UtteranceSegmenter::new(config).expect("config"),
        clock.clone(),
        Arc::new(Counters::default()),
        super::listen::LevelMeter::default(),
    )
}

fn feed(state: &mut ListenState, route: Route, audio: &[f32]) -> Vec<ListenEvent> {
    frames(audio)
        .into_iter()
        .flat_map(|frame| state.on_frame(FrameMsg { frame, route }))
        .collect()
}

fn utterances(events: &[ListenEvent]) -> Vec<&super::msg::UtteranceMsg> {
    events
        .iter()
        .filter_map(|e| match e {
            ListenEvent::Utterance(u) => Some(u),
            _ => None,
        })
        .collect()
}

#[test]
fn an_utterance_ends_after_the_silence_and_the_latency_starts_at_the_last_speech_frame() {
    let clock = ManualClock::new();
    let mut state = listener(&clock);
    let mut events = feed(&mut state, Route::Vad, &speech_audio(640));
    events.extend(feed(&mut state, Route::Vad, &silence_audio(1_000)));

    assert!(
        events
            .iter()
            .any(|e| matches!(e, ListenEvent::SpeechStarted))
    );
    let found = utterances(&events);
    assert_eq!(found.len(), 1, "{events:?}");
    let utterance = found[0];
    // 600 ms of silence is 19 frames of 32 ms.
    assert_eq!(
        utterance.ended - utterance.last_speech,
        Duration::from_millis(19 * 32)
    );
    assert!(utterance.samples.len() >= 640 * 16);
}

#[test]
fn a_short_burst_is_not_an_utterance() {
    let clock = ManualClock::new();
    let mut state = listener(&clock);
    // 100 ms of speech is below the 200 ms minimum.
    let mut events = feed(&mut state, Route::Vad, &speech_audio(100));
    events.extend(feed(&mut state, Route::Vad, &silence_audio(1_500)));
    assert!(utterances(&events).is_empty());
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, ListenEvent::SpeechStarted))
    );
}

#[test]
fn the_gate_closing_drops_the_half_heard_utterance() {
    let clock = ManualClock::new();
    let mut state = listener(&clock);
    let mut events = feed(&mut state, Route::Vad, &speech_audio(640));
    // The gate closes: one marker, then nothing for as long as the tutor speaks.
    events.extend(state.on_frame(FrameMsg {
        frame: Vec::new(),
        route: Route::Dropped,
    }));
    events.extend(feed(&mut state, Route::Vad, &silence_audio(1_000)));
    assert!(utterances(&events).is_empty(), "{events:?}");
}

#[test]
fn push_to_talk_collects_frames_until_release_and_has_no_endpointing_wait() {
    let clock = ManualClock::new();
    let mut state = listener(&clock);
    let mut events = feed(&mut state, Route::PushToTalk, &speech_audio(800));
    assert!(matches!(
        events.first(),
        Some(ListenEvent::PushToTalkStarted)
    ));
    assert!(utterances(&events).is_empty(), "nothing before the release");
    // The first frame that is not push-to-talk is the release.
    events.extend(feed(&mut state, Route::Vad, &silence_audio(64)));
    let found = utterances(&events);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].samples.len(), 800 * 16 / 512 * 512);
    assert_eq!(found[0].ended, found[0].last_speech);
}

#[test]
fn a_tap_on_push_to_talk_is_ignored() {
    let clock = ManualClock::new();
    let mut state = listener(&clock);
    let mut events = feed(&mut state, Route::PushToTalk, &speech_audio(64));
    events.extend(feed(&mut state, Route::Vad, &silence_audio(64)));
    assert!(utterances(&events).is_empty());
}

#[test]
fn a_vad_that_fails_is_reported_with_no_audio_in_the_message() {
    let clock = ManualClock::new();
    let mut state = listener(&clock);
    let events = state.on_frame(FrameMsg {
        frame: vec![0.0; 100],
        route: Route::Vad,
    });
    assert!(matches!(&events[..], [ListenEvent::Failed(message)] if message.contains("512")));
}

// ---- the session phase ------------------------------------------------------

fn cell() -> (PhaseCell, tokio::sync::broadcast::Receiver<VoiceEvent>) {
    let events = EventSink::new(16);
    let rx = events.subscribe();
    (PhaseCell::new(events), rx)
}

fn active(turn: TurnState) -> Phase {
    Phase::Active { turn }
}

#[test]
fn every_change_of_phase_is_published_and_a_refused_event_changes_nothing() {
    let (cell, mut rx) = cell();
    assert_eq!(cell.phase(), active(TurnState::Listening));
    assert_eq!(
        cell.apply(Event::UtteranceEnded).unwrap(),
        active(TurnState::Transcribing)
    );
    assert!(cell.apply(Event::ReplyFinished).is_err());
    assert_eq!(cell.phase(), active(TurnState::Transcribing));
    assert_eq!(
        rx.try_recv().unwrap(),
        VoiceEvent::State(active(TurnState::Transcribing))
    );
    assert!(
        rx.try_recv().is_err(),
        "the refused event published nothing"
    );
}

#[test]
fn a_turn_task_cannot_move_the_session_after_its_epoch_ended() {
    let (cell, _rx) = cell();
    let (epoch, turn) = cell.begin_turn();
    assert_eq!(turn, 1);
    cell.apply(Event::TextSent).unwrap();
    assert_eq!(
        cell.apply_for(epoch, Event::ReplyStarted),
        Some(active(TurnState::Speaking))
    );
    // The learner stops the tutor: the epoch moves on.
    cell.invalidate();
    cell.apply(Event::StopSpeaking).unwrap();
    assert_eq!(cell.apply_for(epoch, Event::ReplyFinished), None);
    assert_eq!(cell.phase(), active(TurnState::Listening));
    assert_eq!(
        cell.turns_completed(),
        0,
        "a stopped turn is not a finished one"
    );
}

#[test]
fn the_voice_loop_drives_the_phases_of_the_tutor_engine_in_this_order() {
    // Table: the events the loop applies for each kind of turn and the phases
    // that follow. The machine itself is tested in tutor-engine; this pins what
    // the loop does with it.
    let turns: [(&str, &[Event], &[Phase]); 4] = [
        (
            "a spoken turn",
            &[
                Event::UtteranceEnded,
                Event::TranscriptReady,
                Event::ReplyStarted,
                Event::ReplyFinished,
            ],
            &[
                active(TurnState::Transcribing),
                active(TurnState::Thinking),
                active(TurnState::Speaking),
                active(TurnState::Listening),
            ],
        ),
        (
            "a typed turn",
            &[Event::TextSent, Event::ReplyStarted, Event::ReplyFinished],
            &[
                active(TurnState::Thinking),
                active(TurnState::Speaking),
                active(TurnState::Listening),
            ],
        ),
        (
            "the learner stops the tutor while it speaks",
            &[Event::TextSent, Event::ReplyStarted, Event::StopSpeaking],
            &[
                active(TurnState::Thinking),
                active(TurnState::Speaking),
                active(TurnState::Listening),
            ],
        ),
        (
            "the learner stops the tutor while it thinks",
            &[Event::TextSent, Event::Pause, Event::Resume],
            &[
                active(TurnState::Thinking),
                Phase::Paused,
                active(TurnState::Listening),
            ],
        ),
    ];
    for (name, events, expected) in turns {
        let (cell, _rx) = cell();
        let seen: Vec<Phase> = events
            .iter()
            .map(|e| {
                cell.apply(*e)
                    .unwrap_or_else(|error| panic!("{name}: {error}"))
            })
            .collect();
        assert_eq!(seen, expected, "{name}");
    }
}

#[test]
fn the_provider_and_engine_side_states_are_reachable_from_a_turn_and_recover() {
    // Table: starting events, the failure, the phase it leads to, the recovery.
    let cases: [(&[Event], Event, Phase, Event); 3] = [
        (
            &[Event::TextSent],
            Event::ProviderFailed,
            Phase::ProviderUnavailable,
            Event::ProviderRecovered,
        ),
        (
            &[Event::UtteranceEnded],
            Event::EngineFailed(EngineFault::SpeechRecognition),
            Phase::EngineError {
                fault: EngineFault::SpeechRecognition,
            },
            Event::EngineRecovered,
        ),
        (
            &[Event::TextSent, Event::ReplyStarted],
            Event::EngineFailed(EngineFault::Playback),
            Phase::EngineError {
                fault: EngineFault::Playback,
            },
            Event::EngineRecovered,
        ),
    ];
    for (setup, failure, failed, recovery) in cases {
        let (cell, _rx) = cell();
        for event in setup {
            cell.apply(*event).unwrap();
        }
        assert_eq!(cell.apply(failure).unwrap(), failed);
        assert_eq!(cell.apply(recovery).unwrap(), active(TurnState::Listening));
    }
}

// ---- the request ---------------------------------------------------------------

fn user(text: &str) -> ChatMessage {
    ChatMessage::user(text)
}

fn assistant(text: &str) -> ChatMessage {
    ChatMessage::assistant(text)
}

#[test]
fn the_request_starts_with_a_learner_message_and_joins_consecutive_ones() {
    let history = vec![
        assistant("cut off by the window"),
        user("one"),
        user("two"),
        assistant("reply"),
    ];
    let messages = build_messages(&history, user("three"));
    let roles: Vec<Role> = messages.iter().map(|m| m.role).collect();
    assert_eq!(roles, [Role::User, Role::Assistant, Role::User]);
    assert_eq!(messages[0].content, "one\ntwo");
    assert_eq!(messages[2].content, "three");
}

#[test]
fn a_current_message_after_an_unanswered_one_is_joined_to_it() {
    let messages = build_messages(&[user("unanswered")], user("again"));
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].content, "unanswered\nagain");
}

#[test]
fn the_request_holds_at_most_the_window_of_history_and_the_current_message() {
    let history: Vec<ChatMessage> = (0..40)
        .map(|i| {
            if i % 2 == 0 {
                user(&format!("u{i}"))
            } else {
                assistant(&format!("a{i}"))
            }
        })
        .collect();
    let messages = build_messages(&history, user("now"));
    assert!(messages.len() <= tutor_engine::HISTORY_MESSAGES + 1);
    assert_eq!(messages.first().map(|m| m.role), Some(Role::User));
    assert_eq!(messages.last().map(|m| m.content.as_str()), Some("now"));
}

#[test]
fn the_opening_instruction_comes_first_and_the_greeting_is_not_lost() {
    // The opening turn stores the instruction as a learner message, so the
    // window never starts with the tutor's own greeting.
    let history = vec![
        user("Begin the conversation now."),
        assistant("Hello! I am Sam."),
    ];
    let messages = build_messages(&history, user("Hi, I am Dewi."));
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[1].content, "Hello! I am Sam.");
}

#[test]
fn the_level_meter_keeps_the_loudest_sample_of_a_window_and_starts_a_new_one_when_read() {
    let meter = super::listen::LevelMeter::default();
    assert!(meter.take() < f32::EPSILON, "silent before any frame");
    meter.record(&[0.1, -0.6, 0.3]);
    meter.record(&[0.2, 0.05]);
    assert!(
        (meter.take() - 0.6).abs() < 1e-6,
        "the peak is by magnitude"
    );
    assert!(meter.take() < f32::EPSILON, "reading starts a new window");
    meter.record(&[3.0, -4.0]);
    assert!(
        (meter.take() - 1.0).abs() < f32::EPSILON,
        "a sample past full scale reads as 1"
    );
}
