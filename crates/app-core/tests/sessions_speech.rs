#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! Speech outside a conversation: speaking a stored text, the audio devices and
//! the microphone test. The engines are fakes; nothing here touches hardware.

mod common;

use std::time::Duration;

use app_core::api::{
    AudioTestRequest, EngineId, EngineState, ErrorCode, Feature, GenerateReadingRequest,
    ReadingOutcomeView, ServerEvent, SessionKind, SpeakRequest, TopicChoice,
};
use app_core::error::CoreError;
use app_core::voice::testing::{ReplyScript, Step};
use common::llm::{ContractLlm, reading_text};
use common::sessions::{SessionRig, Setup, chat_request, request, rig, unit_request};

fn reply(text: &str) -> Step {
    Step::Reply(ReplyScript::new(&[text]))
}

/// Starts a chat and waits for the tutor's opening line to be stored. Returns the
/// session id and the stored position of that line.
async fn chat_with_opening(r: &SessionRig) -> (i64, i64) {
    let view = r.start(chat_request("food")).await;
    let events = r
        .events
        .wait("the opening line to be stored", |events| {
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
    let seq = events
        .iter()
        .find_map(|e| match e {
            ServerEvent::TurnState {
                tutor_turn_seq: Some(seq),
                ..
            } => Some(*seq),
            _ => None,
        })
        .unwrap();
    (view.id, seq)
}

async fn spoken(r: &SessionRig, count: usize) -> Vec<String> {
    let speech = r.speech.as_ref().expect("fake speech");
    for _ in 0..2_000 {
        let spoken = speech.tts_log.spoken.lock().unwrap().clone();
        if spoken.len() >= count {
            return spoken;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("timed out waiting for {count} spoken sentences");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stored_tutor_turn_is_spoken_by_reference_and_the_synthesiser_shows_as_ready() {
    let r = rig(Setup {
        steps: vec![reply("Hello! What is your name?")],
        speech: Some(Vec::new()),
        ..Setup::default()
    })
    .await;
    let (id, seq) = chat_with_opening(&r).await;
    let engine = |r: &SessionRig| {
        r.core()
            .snapshot()
            .engines
            .into_iter()
            .find(|e| e.id == EngineId::Tts)
            .unwrap()
    };
    assert_eq!(
        engine(&r).state,
        EngineState::Configured,
        "nothing loaded yet"
    );

    let accepted = r
        .core()
        .speak(SpeakRequest::Turn {
            session_id: id,
            turn_seq: seq,
        })
        .await
        .expect("spoken");
    assert_eq!(accepted.sentences, 2);
    let said = spoken(&r, 2).await;
    assert_eq!(said, ["Hello!", "What is your name?"]);

    let tts = engine(&r);
    assert_eq!(tts.state, EngineState::Ready);
    assert!(tts.model.is_some(), "the engine says which model it loaded");
    let names = r.events.names();
    assert!(names.contains(&"EngineStatus"), "{names:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn only_text_the_server_stored_can_be_spoken() {
    let r = rig(Setup {
        steps: vec![reply("Hello!"), reply("Good.")],
        speech: Some(Vec::new()),
        ..Setup::default()
    })
    .await;
    let (id, seq) = chat_with_opening(&r).await;
    r.core()
        .send_text(id, "My own words.".to_owned())
        .await
        .unwrap();
    r.events.wait_for("AnalysisReady", 0).await;

    // A turn that does not exist, a session that does not exist.
    for request in [
        SpeakRequest::Turn {
            session_id: id,
            turn_seq: 999,
        },
        SpeakRequest::Turn {
            session_id: 9_999,
            turn_seq: seq,
        },
        SpeakRequest::Reading {
            session_id: id,
            content_id: 1,
        },
    ] {
        let refused = r.core().speak(request).await.unwrap_err();
        assert!(matches!(refused, CoreError::NotFound { .. }), "{refused:?}");
    }
    // The learner's own line is not the tutor's to speak.
    let learner_seq = seq + 1;
    let refused = r
        .core()
        .speak(SpeakRequest::Turn {
            session_id: id,
            turn_seq: learner_seq,
        })
        .await
        .unwrap_err();
    assert!(matches!(refused, CoreError::InvalidInput(_)), "{refused:?}");
    // An activity of a session that is not a unit run.
    let refused = r
        .core()
        .speak(SpeakRequest::Activity {
            session_id: id,
            activity_id: "a06-dictation-from".to_owned(),
        })
        .await
        .unwrap_err();
    assert!(matches!(refused, CoreError::Conflict(_)), "{refused:?}");

    let speech = r.speech.as_ref().unwrap();
    assert!(
        speech.tts_log.spoken.lock().unwrap().is_empty(),
        "nothing was spoken for a refused request"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_request_while_speaking_is_refused_and_stop_speaking_frees_the_output() {
    let r = rig(Setup {
        steps: vec![reply(
            "This is a long line that takes a while to say aloud. It has two sentences.",
        )],
        speech: Some(Vec::new()),
        ..Setup::default()
    })
    .await;
    let (id, seq) = chat_with_opening(&r).await;
    let turn = SpeakRequest::Turn {
        session_id: id,
        turn_seq: seq,
    };
    r.core().speak(turn.clone()).await.unwrap();
    spoken(&r, 1).await;
    let busy = r.core().speak(turn.clone()).await.unwrap_err();
    assert!(matches!(busy, CoreError::Busy), "{busy:?}");

    let ack = r.core().stop_speaking().await.unwrap();
    assert!(ack.ok);
    let mut taken = false;
    for _ in 0..400 {
        if r.core().speak(turn.clone()).await.is_ok() {
            taken = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(taken, "after a stop the output takes a new text");
    r.core().stop_speaking().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn stopping_the_speech_when_nothing_speaks_is_not_an_error() {
    let r = rig(Setup::default()).await;
    assert!(r.core().stop_speaking().await.unwrap().ok);
    let with = rig(Setup {
        speech: Some(Vec::new()),
        ..Setup::default()
    })
    .await;
    assert!(with.core().stop_speaking().await.unwrap().ok);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_generated_reading_text_can_be_read_aloud() {
    let r = rig(Setup {
        speech: Some(Vec::new()),
        ..Setup::default()
    })
    .await;
    let llm = ContractLlm::new(r.llm.clone());
    llm.queue_reading(reading_text());
    r.manager.use_llm(llm);
    let view = r.start(request(SessionKind::Reading)).await;
    let ReadingOutcomeView::Generated { content_id, .. } = r
        .core()
        .generate_reading(GenerateReadingRequest {
            session_id: view.id,
            topic: TopicChoice::Typed {
                text: "the market".to_owned(),
            },
        })
        .await
        .unwrap()
    else {
        panic!("a text");
    };
    let accepted = r
        .core()
        .speak(SpeakRequest::Reading {
            session_id: view.id,
            content_id,
        })
        .await
        .unwrap();
    assert_eq!(accepted.sentences, 12);
    let said = spoken(&r, 1).await;
    assert_eq!(said[0], "The market sells fresh fruit every morning.");
    r.core().stop_speaking().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_audio_of_a_listening_item_is_read_from_the_run_and_each_request_counts_as_a_play() {
    let r = rig(Setup {
        speech: Some(Vec::new()),
        ..Setup::default()
    })
    .await;
    let view = r.start(unit_request(SessionKind::Lesson, "a1-u01")).await;
    let item = SpeakRequest::Activity {
        session_id: view.id,
        activity_id: "a15-listen-set-putu".to_owned(),
    };
    // Two replays are allowed after the first play.
    let mut plays = 0;
    for _ in 0..3 {
        for _ in 0..600 {
            match r.core().speak(item.clone()).await {
                Ok(_) => {
                    plays += 1;
                    break;
                }
                Err(CoreError::Busy) => tokio::time::sleep(Duration::from_millis(10)).await,
                Err(other) => panic!("a play within the allowance: {other:?}"),
            }
        }
        r.core().stop_speaking().await.unwrap();
    }
    assert_eq!(plays, 3, "a busy refusal did not count as a play");
    let mut fourth = None;
    for _ in 0..600 {
        match r.core().speak(item.clone()).await {
            Err(CoreError::Busy) => tokio::time::sleep(Duration::from_millis(10)).await,
            other => {
                fourth = Some(other);
                break;
            }
        }
    }
    let refused = fourth.expect("an answer").unwrap_err();
    assert!(
        matches!(refused, CoreError::InvalidInput(_) | CoreError::Conflict(_)),
        "no plays are left: {refused:?}"
    );
    let said = spoken(&r, 1).await;
    assert!(said[0].starts_with("Hello!"), "{said:?}");
    let dictation = r
        .core()
        .speak(SpeakRequest::Activity {
            session_id: view.id,
            activity_id: "a02-greeting-by-time".to_owned(),
        })
        .await
        .unwrap_err();
    assert!(
        matches!(
            dictation,
            CoreError::InvalidInput(_) | CoreError::Conflict(_)
        ),
        "an item with no audio has nothing to play: {dictation:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn without_audio_or_speech_the_routes_answer_not_available_and_name_what_is_missing() {
    let r = rig(Setup {
        steps: vec![reply("Hello!")],
        ..Setup::default()
    })
    .await;
    let (id, seq) = chat_with_opening(&r).await;

    let speak = r
        .core()
        .speak(SpeakRequest::Turn {
            session_id: id,
            turn_seq: seq,
        })
        .await
        .unwrap_err();
    let body = speak.body();
    assert_eq!(body.error, ErrorCode::NotAvailable);
    assert_eq!(body.feature, None);
    assert!(
        body.message.contains("audio devices") && body.message.contains("no audio"),
        "{}",
        body.message
    );

    let devices = r.core().audio_devices().await.unwrap_err();
    assert_eq!(devices.body().error, ErrorCode::NotAvailable);
    assert_eq!(devices.body().feature, Some(Feature::Speech));

    let test = r
        .core()
        .audio_test(AudioTestRequest {
            duration_ms: 500,
            play_back: false,
            input_id: None,
            output_id: None,
            remember: false,
        })
        .await
        .unwrap_err();
    assert_eq!(test.body().error, ErrorCode::NotAvailable);
    assert_eq!(test.body().feature, Some(Feature::Speech));

    // The rest of the program works.
    r.core()
        .send_text(id, "Hello.".to_owned())
        .await
        .expect("text chat works without speech");
    r.events.wait_for("AnalysisReady", 0).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_devices_are_listed_with_the_default_and_the_chosen_one_marked() {
    let r = rig(Setup {
        speech: Some(Vec::new()),
        ..Setup::default()
    })
    .await;
    let devices = r.core().audio_devices().await.unwrap();
    assert_eq!(devices.inputs.len(), 1);
    assert_eq!(devices.outputs.len(), 1);
    assert_eq!(devices.inputs[0].name, "Fake microphone");
    assert_eq!(devices.inputs[0].sample_rate, 16_000);
    assert_eq!(devices.inputs[0].channels, 1);
    assert!(devices.inputs[0].is_default);
    assert_eq!(devices.outputs[0].name, "Fake speaker");
    assert_eq!(devices.outputs[0].channels, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_microphone_test_reports_what_it_heard_at_most_ten_levels_a_second_and_stores_nothing()
{
    let r = rig(Setup {
        speech: Some(Vec::new()),
        ..Setup::default()
    })
    .await;
    let backend = r.audio.as_ref().unwrap().backend.inner.clone();
    backend.set_input_tone(Some(440.0));
    let mark = r.mark().await;
    let started = std::time::Instant::now();
    let report = r
        .core()
        .audio_test(AudioTestRequest {
            duration_ms: 700,
            play_back: false,
            input_id: None,
            output_id: None,
            remember: false,
        })
        .await
        .expect("the test runs");
    let took = started.elapsed();
    backend.set_input_tone(None);
    assert_eq!(report.input_device, "Fake microphone");
    assert!(report.peak > 0.2, "a tone was heard: {report:?}");
    assert!(!report.silent);
    assert!(report.recorded_ms > 300, "{report:?}");
    assert!(!report.played_back);

    let events = r.events.all();
    let levels: Vec<f32> = events
        .iter()
        .skip(mark)
        .filter_map(|e| match e {
            ServerEvent::MicLevel { level, .. } => Some(*level),
            _ => None,
        })
        .collect();
    assert!(!levels.is_empty());
    assert!(
        levels.len() as f64 <= took.as_secs_f64() * 10.0 + 0.5,
        "{} levels in {took:?}",
        levels.len()
    );
    assert!(levels.iter().any(|l| *l > 0.2));
    let status = r
        .core()
        .snapshot()
        .engines
        .into_iter()
        .find(|e| e.id == EngineId::AudioInput)
        .unwrap();
    assert_eq!(status.state, EngineState::Ready);

    // Nothing of the recording is kept: no file in the data directory.
    let mut files = Vec::new();
    let mut stack = vec![r.core().config().data_dir.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                files.push(path);
            }
        }
    }
    assert!(
        files.iter().all(|f| f
            .extension()
            .is_none_or(|e| e != "wav" && e != "pcm" && e != "raw")),
        "{files:?}"
    );

    // A silent room is reported, not guessed.
    let quiet = r
        .core()
        .audio_test(AudioTestRequest {
            duration_ms: 500,
            play_back: false,
            input_id: None,
            output_id: None,
            remember: false,
        })
        .await
        .unwrap();
    assert!(quiet.silent, "{quiet:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_microphone_test_checks_its_length_its_devices_and_runs_alone() {
    let r = rig(Setup {
        speech: Some(Vec::new()),
        ..Setup::default()
    })
    .await;
    for ms in [0, 499, 5_001, 60_000] {
        let refused = r
            .core()
            .audio_test(AudioTestRequest {
                duration_ms: ms,
                play_back: false,
                input_id: None,
                output_id: None,
                remember: false,
            })
            .await
            .unwrap_err();
        assert!(
            matches!(refused, CoreError::InvalidInput(_)),
            "{ms}: {refused:?}"
        );
    }
    let unknown = r
        .core()
        .audio_test(AudioTestRequest {
            duration_ms: 500,
            play_back: false,
            input_id: Some("no-such-device".to_owned()),
            output_id: None,
            remember: false,
        })
        .await
        .unwrap_err();
    assert!(matches!(unknown, CoreError::InvalidInput(_)), "{unknown:?}");

    let core = std::sync::Arc::clone(r.core());
    let first = tokio::spawn(async move {
        core.audio_test(AudioTestRequest {
            duration_ms: 800,
            play_back: false,
            input_id: None,
            output_id: None,
            remember: false,
        })
        .await
    });
    tokio::time::sleep(Duration::from_millis(150)).await;
    let second = r
        .core()
        .audio_test(AudioTestRequest {
            duration_ms: 500,
            play_back: false,
            input_id: None,
            output_id: None,
            remember: false,
        })
        .await
        .unwrap_err();
    assert!(matches!(second, CoreError::Busy), "{second:?}");
    first.await.unwrap().expect("the first test finishes");
}
