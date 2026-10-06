//! The sherpa-onnx engines, named by the engines file. UNVERIFIED: this compiles
//! with the `sherpa` feature but has never loaded a model, because the build
//! container cannot download the native library or the models.

use std::path::Path;

use speech::{
    SherpaStt, SherpaSttConfig, SherpaTts, SherpaTtsConfig, SherpaVad, SherpaVadConfig, SttEngine,
    TtsEngine,
};

use super::file::{self, EnginesFile};
use super::{EngineOptions, EngineProblem, PartStatus, SpeechSource, SpeechStatus};
use crate::voice::{ListenParts, TtsLoad};

/// The default name of the engines file in the data directory.
pub const ENGINES_FILE: &str = "engines.toml";

/// The speech engines an engines file names.
pub struct SherpaSource {
    vad: Option<SherpaVadConfig>,
    stt: Option<SherpaSttConfig>,
    tts: Option<SherpaTtsConfig>,
}

/// Reads the engines file of a run. `Err` is the reason there are no speech engines.
pub(super) fn source(options: &EngineOptions) -> Result<SherpaSource, String> {
    let path = options
        .engines_file
        .clone()
        .unwrap_or_else(|| options.data_dir.join(ENGINES_FILE));
    SherpaSource::from_file(&path, Some(&options.data_dir.join("models")))
}

impl SherpaSource {
    /// Reads an engines file. A path in it that is not absolute is resolved
    /// against `base`. `Err` is a sentence without a path.
    pub fn from_file(path: &Path, base: Option<&Path>) -> Result<Self, String> {
        let text = read(path)?;
        let EnginesFile { vad, stt, tts } = file::parse(&text, base)?;
        Ok(Self { vad, stt, tts })
    }
}

fn read(path: &Path) -> Result<String, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(
            "no engines file names the speech models (engines.toml in the data folder)".to_owned(),
        ),
        Err(error) => Err(format!(
            "the engines file cannot be read ({:?})",
            error.kind()
        )),
    }
}

fn status_of<E>(
    section: &Option<E>,
    name: &str,
    validate: impl Fn(&E) -> Result<(), speech::SherpaConfigError>,
) -> PartStatus {
    match section {
        None => PartStatus::Missing(format!("the engines file has no [{name}] section")),
        Some(config) => match validate(config) {
            Ok(()) => PartStatus::Configured,
            Err(error) => PartStatus::Missing(file::sentence(&error)),
        },
    }
}

impl SpeechSource for SherpaSource {
    fn status(&self) -> SpeechStatus {
        SpeechStatus {
            vad: status_of(&self.vad, "vad", SherpaVadConfig::validate),
            stt: status_of(&self.stt, "stt", SherpaSttConfig::validate),
            tts: status_of(&self.tts, "tts", SherpaTtsConfig::validate),
        }
    }

    fn listen(&self) -> Result<ListenParts, EngineProblem> {
        let vad_config = self
            .vad
            .clone()
            .ok_or_else(|| EngineProblem("the engines file has no [vad] section".to_owned()))?;
        let stt_config = self
            .stt
            .clone()
            .ok_or_else(|| EngineProblem("the engines file has no [stt] section".to_owned()))?;
        let vad = SherpaVad::load(&vad_config)
            .map_err(|error| EngineProblem(format!("the VAD did not load: {error}")))?;
        Ok(ListenParts {
            vad: Box::new(vad),
            stt: Box::new(move || {
                SherpaStt::load(&stt_config).map(|engine| Box::new(engine) as Box<dyn SttEngine>)
            }),
        })
    }

    fn tts(&self) -> Result<TtsLoad, EngineProblem> {
        let config = self
            .tts
            .clone()
            .ok_or_else(|| EngineProblem("the engines file has no [tts] section".to_owned()))?;
        Ok(Box::new(move || {
            SherpaTts::load(&config).map(|engine| Box::new(engine) as Box<dyn TtsEngine>)
        }))
    }
}
