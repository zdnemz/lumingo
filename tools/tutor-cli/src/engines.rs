//! Which audio backend and which speech engines a run uses.
//!
//! A default build has neither a real audio backend nor real speech engines:
//! asking for voice then fails with a message that names the cargo feature to
//! build with. Fakes exist only in builds with the `test-support` feature.

use std::path::Path;

use app_core::voice::{AudioMode, ListenParts, TtsLoad};

use crate::args::BackendArg;

/// What a run needs from the audio and speech side.
#[derive(Debug, Clone, Copy)]
pub struct Need {
    /// The microphone is the input.
    pub microphone: bool,
    /// A VAD and a recogniser are needed: the microphone or recorded utterances.
    pub listen: bool,
    /// The tutor's replies are spoken.
    pub speak: bool,
}

/// What the loop is started with.
pub struct Built {
    pub audio: AudioMode,
    pub listen: Option<ListenParts>,
    pub tts: Option<TtsLoad>,
    pub backend: &'static str,
    /// Something that belongs next to any number from this run.
    pub warning: Option<String>,
}

/// Why the audio and speech side cannot be built. The message is for the person
/// at the keyboard.
#[derive(Debug)]
pub struct Problem(pub String);

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Problem {}

pub struct Settings<'a> {
    pub backend: BackendArg,
    pub engines_file: Option<&'a Path>,
    pub data_dir: Option<&'a Path>,
    pub end_silence_ms: u32,
    /// How many scripted transcripts a fake recogniser should have.
    pub fake_transcripts: usize,
}

/// Builds the audio and speech side. This loads models for the VAD, so call it
/// where blocking is allowed.
pub fn build(need: Need, settings: &Settings<'_>) -> Result<Built, Problem> {
    if !need.listen && !need.speak {
        return Ok(Built {
            audio: AudioMode::None,
            listen: None,
            tts: None,
            backend: "none",
            warning: None,
        });
    }
    match settings.backend {
        BackendArg::Fake => fake(need, settings),
        BackendArg::Cpal => real(need, settings),
    }
}

#[cfg(feature = "test-support")]
fn fake(need: Need, settings: &Settings<'_>) -> Result<Built, Problem> {
    use app_core::voice::testing::{EnergyVad, FakeStt, FakeTts, fake_audio};

    let audio = fake_audio();
    let mode = if need.microphone {
        AudioMode::Full(audio.registry)
    } else if need.speak {
        AudioMode::PlaybackOnly(audio.registry)
    } else {
        AudioMode::None
    };
    let listen = need.listen.then(|| {
        let texts: Vec<String> = (1..=settings.fake_transcripts.max(1))
            .map(|n| format!("fake transcript {n}"))
            .collect();
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        let stt = FakeStt::new(&refs, None, std::time::Duration::ZERO);
        ListenParts {
            vad: Box::new(EnergyVad::new(None, settings.end_silence_ms)),
            stt: Box::new(move || Ok(Box::new(stt) as _)),
        }
    });
    let tts: Option<TtsLoad> = need.speak.then(|| {
        let (tts, _log) = FakeTts::new(None, std::time::Duration::ZERO);
        let load: TtsLoad = Box::new(move || Ok(Box::new(tts) as _));
        load
    });
    Ok(Built {
        audio: mode,
        listen,
        tts,
        backend: "fake",
        warning: Some(
            "fake devices and fake speech engines: these numbers say nothing about real speech"
                .to_owned(),
        ),
    })
}

#[cfg(not(feature = "test-support"))]
fn fake(_need: Need, _settings: &Settings<'_>) -> Result<Built, Problem> {
    Err(Problem(
        "this build has no fake backend: it is only compiled with --features test-support"
            .to_owned(),
    ))
}

fn real(need: Need, settings: &Settings<'_>) -> Result<Built, Problem> {
    let (listen, tts) = speech_engines(need, settings)?;
    let audio = devices(need, settings)?;
    Ok(Built {
        audio,
        listen,
        tts,
        backend: if need.microphone || need.speak {
            "cpal"
        } else {
            "none"
        },
        warning: None,
    })
}

#[cfg(feature = "cpal-backend")]
fn devices(need: Need, settings: &Settings<'_>) -> Result<AudioMode, Problem> {
    if !need.microphone && !need.speak {
        return Ok(AudioMode::None);
    }
    let dir = settings
        .data_dir
        .map(Path::to_owned)
        .or_else(app_core::default_data_dir);
    let registry = crate::cpal_devices::registry(dir.as_deref());
    Ok(if need.microphone {
        AudioMode::Full(registry)
    } else {
        AudioMode::PlaybackOnly(registry)
    })
}

#[cfg(not(feature = "cpal-backend"))]
fn devices(need: Need, _settings: &Settings<'_>) -> Result<AudioMode, Problem> {
    if need.microphone || need.speak {
        return Err(Problem(
            "this build has no audio backend: build with --features cpal-backend (UNVERIFIED on \
             hardware), or use --text without --speak"
                .to_owned(),
        ));
    }
    Ok(AudioMode::None)
}

#[cfg(feature = "sherpa")]
fn speech_engines(
    need: Need,
    settings: &Settings<'_>,
) -> Result<(Option<ListenParts>, Option<TtsLoad>), Problem> {
    crate::sherpa::load(need, settings.engines_file)
}

#[cfg(not(feature = "sherpa"))]
fn speech_engines(
    _need: Need,
    _settings: &Settings<'_>,
) -> Result<(Option<ListenParts>, Option<TtsLoad>), Problem> {
    Err(Problem(
        "speech models need the sherpa feature (UNVERIFIED): build with --features sherpa"
            .to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(backend: BackendArg) -> Settings<'static> {
        Settings {
            backend,
            engines_file: None,
            data_dir: None,
            end_silence_ms: 600,
            fake_transcripts: 3,
        }
    }

    #[test]
    fn typed_input_without_speech_needs_nothing() {
        let need = Need {
            microphone: false,
            listen: false,
            speak: false,
        };
        let built = build(need, &settings(BackendArg::Cpal)).ok().unwrap();
        assert!(matches!(built.audio, AudioMode::None));
        assert!(built.listen.is_none() && built.tts.is_none());
        assert_eq!(built.backend, "none");
    }

    #[cfg(not(any(feature = "cpal-backend", feature = "sherpa")))]
    #[test]
    fn voice_in_a_default_build_says_which_feature_to_build_with() {
        let need = Need {
            microphone: true,
            listen: true,
            speak: true,
        };
        let problem = build(need, &settings(BackendArg::Cpal)).err().unwrap();
        assert!(problem.0.contains("--features sherpa"), "{problem}");
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn the_fake_backend_is_marked_so_its_numbers_are_not_mistaken_for_real_ones() {
        let need = Need {
            microphone: false,
            listen: true,
            speak: true,
        };
        let built = build(need, &settings(BackendArg::Fake)).ok().unwrap();
        assert!(matches!(built.audio, AudioMode::PlaybackOnly(_)));
        assert!(built.warning.unwrap().contains("fake"));
    }

    #[cfg(not(feature = "test-support"))]
    #[test]
    fn the_fake_backend_is_refused_in_a_build_without_test_support() {
        let need = Need {
            microphone: false,
            listen: true,
            speak: true,
        };
        let problem = build(need, &settings(BackendArg::Fake)).err().unwrap();
        assert!(problem.0.contains("test-support"));
    }
}
