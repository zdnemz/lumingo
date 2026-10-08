#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! Which engines a run has, and how the program says so when it has none.

use app_core::api::{EngineId, EngineState};
use app_core::engines::testing::fake_engines;
use app_core::engines::{EngineOptions, Engines};

#[test]
fn engines_that_were_not_built_say_why_for_every_engine() {
    let engines = Engines::without("this run has no engines");
    assert!(engines.audio().is_err());
    assert!(engines.speech().is_err());
    assert!(engines.drill().is_none());
    assert!(!engines.speech_ready());
    let views = engines.views();
    let ids: Vec<EngineId> = views.iter().map(|v| v.id).collect();
    assert_eq!(
        ids,
        [
            EngineId::AudioInput,
            EngineId::AudioOutput,
            EngineId::Vad,
            EngineId::Stt,
            EngineId::Tts,
            EngineId::Pron
        ]
    );
    for view in &views {
        assert_eq!(view.state, EngineState::Unavailable, "{:?}", view.id);
        assert!(!view.detail.is_empty());
        assert!(view.model.is_none());
    }
    assert!(views[0].detail.contains("this run has no engines"));
    assert!(views[5].detail.contains("pronunciation"));
}

/// A build with every feature off, which is what the real server binary is by
/// default, has no audio and no speech, and has no fake in its place.
#[cfg(not(any(feature = "cpal-backend", feature = "sherpa")))]
#[test]
fn detection_in_a_build_with_every_feature_off_finds_nothing_and_names_the_features() {
    let dir = tempfile::tempdir().unwrap();
    let engines = Engines::detect(&EngineOptions {
        data_dir: dir.path().to_owned(),
        engines_file: None,
    });
    assert!(!engines.speech_ready());
    let Err(audio) = engines.audio() else {
        panic!("a build without cpal-backend has audio");
    };
    let Err(speech) = engines.speech() else {
        panic!("a build without sherpa has speech");
    };
    assert!(audio.contains("cpal-backend"), "{audio}");
    assert!(speech.contains("sherpa"), "{speech}");
    for view in engines.views() {
        assert_eq!(view.state, EngineState::Unavailable);
    }
}

#[test]
fn fake_engines_are_configured_and_ready_for_a_voice_session() {
    let fake = fake_engines(&["hello"]);
    assert!(fake.engines.speech_ready());
    let views = fake.engines.views();
    for id in [
        EngineId::AudioInput,
        EngineId::AudioOutput,
        EngineId::Vad,
        EngineId::Stt,
        EngineId::Tts,
    ] {
        let view = views.iter().find(|v| v.id == id).unwrap();
        assert_eq!(view.state, EngineState::Configured, "{id:?}");
    }
    assert_eq!(
        views.iter().find(|v| v.id == EngineId::Pron).unwrap().state,
        EngineState::Unavailable,
        "no phoneme model, so pronunciation is not scored"
    );
}
