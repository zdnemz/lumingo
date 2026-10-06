//! Which audio devices and speech engines this run has.
//!
//! The real ones are behind cargo features that are off by default:
//!
//! * `cpal-backend` (microphone and speakers through cpal, UNVERIFIED on hardware)
//! * `sherpa` (VAD, speech recognition and synthesis through sherpa-onnx,
//!   UNVERIFIED: the native library cannot be built in the container)
//!
//! [`Engines::detect`] builds what the features and the files on disk allow, and
//! says why for everything it could not build. A build with every feature off has
//! no audio and no speech, and the program says so: text chat, lessons, writing
//! and reading work without them. Fakes for tests live in [`testing`], compiled
//! only for this crate's tests and with the `test-support` feature.

use std::path::PathBuf;
use std::sync::Arc;

use audio_io::DeviceRegistry;
use tutor_engine::DrillScorer;

use crate::api::{EngineId, EngineState, EngineView};
use crate::voice::{ListenParts, TtsLoad};

#[cfg(feature = "cpal-backend")]
mod cpal;
pub mod file;
#[cfg(feature = "sherpa")]
mod sherpa;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;

#[cfg(feature = "sherpa")]
pub use sherpa::SherpaSource;

/// Why an engine could not be built or loaded. The text is a sentence for the
/// learner and holds no path and no learner text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct EngineProblem(pub String);

/// Where one part of the speech side stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PartStatus {
    /// Named by the engines file and its files are there.
    Configured,
    /// Not named, or a file is missing. The sentence says which.
    Missing(String),
}

/// The three speech engines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeechStatus {
    pub vad: PartStatus,
    pub stt: PartStatus,
    pub tts: PartStatus,
}

/// A place speech engines come from. Each call builds new engines: a voice
/// session owns its engines and drops them when it ends.
pub trait SpeechSource: Send + Sync {
    /// What is configured, without loading a model. It looks at files, so it is
    /// not for the async runtime.
    fn status(&self) -> SpeechStatus;

    /// A VAD, loaded now on the calling thread, and a loader for the recogniser
    /// that runs on the recogniser's own thread. Blocking.
    fn listen(&self) -> Result<ListenParts, EngineProblem>;

    /// A loader for the synthesiser, which runs on the synthesiser's own thread.
    fn tts(&self) -> Result<TtsLoad, EngineProblem>;
}

/// What [`Engines::detect`] reads.
#[derive(Debug, Clone)]
pub struct EngineOptions {
    /// The app data directory: device choices are remembered here, and the default
    /// engines file and the models folder are in it.
    pub data_dir: PathBuf,
    /// The engines file. Without one, `engines.toml` in the data directory.
    pub engines_file: Option<PathBuf>,
}

/// The audio and speech side of one run.
#[derive(Clone)]
pub struct Engines {
    audio: Result<Arc<DeviceRegistry>, String>,
    speech: Result<Arc<dyn SpeechSource>, String>,
    drill: Option<Arc<dyn DrillScorer>>,
}

impl std::fmt::Debug for Engines {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engines")
            .field("audio", &self.audio.is_ok())
            .field("speech", &self.speech.is_ok())
            .field("drill", &self.drill.is_some())
            .finish()
    }
}

impl Engines {
    /// From parts the caller built. `Err` carries the reason a part is missing.
    pub fn new(
        audio: Result<Arc<DeviceRegistry>, String>,
        speech: Result<Arc<dyn SpeechSource>, String>,
        drill: Option<Arc<dyn DrillScorer>>,
    ) -> Self {
        Self {
            audio,
            speech,
            drill,
        }
    }

    /// Nothing: no audio, no speech, no pronunciation model, each with its reason.
    pub fn without(reason: &str) -> Self {
        Self::new(Err(reason.to_owned()), Err(reason.to_owned()), None)
    }

    /// What the cargo features and the files on disk allow. Blocking: it reads
    /// the engines file and enumerates nothing else. Run it on a blocking thread.
    pub fn detect(options: &EngineOptions) -> Self {
        Self::new(detect_audio(options), detect_speech(options), None)
    }

    /// The audio devices, or the reason there are none.
    pub fn audio(&self) -> Result<&Arc<DeviceRegistry>, &str> {
        self.audio.as_ref().map_err(String::as_str)
    }

    /// The speech engines, or the reason there are none.
    pub fn speech(&self) -> Result<&Arc<dyn SpeechSource>, &str> {
        self.speech.as_ref().map_err(String::as_str)
    }

    /// The pronunciation scorer, when one is loaded.
    pub fn drill(&self) -> Option<&Arc<dyn DrillScorer>> {
        self.drill.as_ref()
    }

    /// Where every engine stands, for the snapshot. It looks at files for the
    /// speech engines, so it is not for the async runtime.
    pub fn views(&self) -> Vec<EngineView> {
        let audio = |id, noun: &str| match &self.audio {
            Ok(_) => EngineView {
                id,
                state: EngineState::Configured,
                detail: format!(
                    "an audio backend is built in; the {noun} is chosen when it is used"
                ),
                model: None,
            },
            Err(reason) => EngineView {
                id,
                state: EngineState::Unavailable,
                detail: reason.clone(),
                model: None,
            },
        };
        let part = |id, status: &PartStatus| match status {
            PartStatus::Configured => EngineView {
                id,
                state: EngineState::Configured,
                detail: "configured; the model loads when a voice session starts".to_owned(),
                model: None,
            },
            PartStatus::Missing(reason) => EngineView {
                id,
                state: EngineState::Unavailable,
                detail: reason.clone(),
                model: None,
            },
        };
        let status = match &self.speech {
            Ok(source) => source.status(),
            Err(reason) => SpeechStatus {
                vad: PartStatus::Missing(reason.clone()),
                stt: PartStatus::Missing(reason.clone()),
                tts: PartStatus::Missing(reason.clone()),
            },
        };
        vec![
            audio(EngineId::AudioInput, "microphone"),
            audio(EngineId::AudioOutput, "speaker"),
            part(EngineId::Vad, &status.vad),
            part(EngineId::Stt, &status.stt),
            part(EngineId::Tts, &status.tts),
            match &self.drill {
                Some(drill) => {
                    let info = drill.engine();
                    EngineView {
                        id: EngineId::Pron,
                        state: EngineState::Configured,
                        detail: format!("the pronunciation scorer {info} is loaded"),
                        model: None,
                    }
                }
                None => EngineView {
                    id: EngineId::Pron,
                    state: EngineState::Unavailable,
                    detail: "no phoneme model is configured, so pronunciation is not scored"
                        .to_owned(),
                    model: None,
                },
            },
        ]
    }

    /// Whether the speech side can run at all: an audio backend and at least
    /// one of recognition and synthesis. It looks at files.
    pub fn speech_ready(&self) -> bool {
        let Ok(source) = &self.speech else {
            return false;
        };
        let status = source.status();
        self.audio.is_ok()
            && (status.vad == PartStatus::Configured && status.stt == PartStatus::Configured
                || status.tts == PartStatus::Configured)
    }
}

#[cfg(feature = "cpal-backend")]
fn detect_audio(options: &EngineOptions) -> Result<Arc<DeviceRegistry>, String> {
    Ok(cpal::registry(Some(&options.data_dir)))
}

#[cfg(not(feature = "cpal-backend"))]
fn detect_audio(_options: &EngineOptions) -> Result<Arc<DeviceRegistry>, String> {
    Err("this build has no audio backend: it was built without the cpal-backend feature".to_owned())
}

#[cfg(feature = "sherpa")]
fn detect_speech(options: &EngineOptions) -> Result<Arc<dyn SpeechSource>, String> {
    sherpa::source(options).map(|source| Arc::new(source) as Arc<dyn SpeechSource>)
}

#[cfg(not(feature = "sherpa"))]
fn detect_speech(_options: &EngineOptions) -> Result<Arc<dyn SpeechSource>, String> {
    Err("this build has no speech engines: it was built without the sherpa feature".to_owned())
}
