//! The sherpa-onnx engines, named by a TOML file. UNVERIFIED: this compiles
//! with the `sherpa` feature but has never loaded a model, because the build
//! container cannot download the native library or the models.
//!
//! ```toml
//! [vad]
//! model = "models/files/silero_vad.onnx"
//!
//! [stt]
//! family = "whisper"            # or "moonshine-v2", or "nemo-transducer"
//! encoder = "models/files/whisper-encoder.onnx"
//! decoder = "models/files/whisper-decoder.onnx"
//! tokens = "models/files/whisper-tokens.txt"
//! language = "en"
//! threads = 4
//!
//! [tts]                         # only needed when the tutor speaks
//! family = "kokoro"             # or "kitten", or "supertonic"
//! model = "..."
//! ```
//!
//! Paths are checked by the speech crate before the native library sees them.

use std::path::{Path, PathBuf};

use app_core::voice::{ListenParts, TtsLoad};
use serde::Deserialize;
use speech::{
    SherpaStt, SherpaSttConfig, SherpaSttModel, SherpaTts, SherpaTtsConfig, SherpaTtsModel,
    SherpaVad, SherpaVadConfig, SttEngine, TtsEngine,
};

use crate::engines::{Need, Problem};

fn default_threads() -> i32 {
    4
}

fn default_speed() -> f32 {
    1.0
}

#[derive(Debug, Deserialize)]
struct File {
    vad: Option<VadSection>,
    stt: Option<SttSection>,
    tts: Option<TtsSection>,
}

#[derive(Debug, Deserialize)]
struct VadSection {
    model: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "family", rename_all = "kebab-case")]
enum SttSection {
    Whisper {
        encoder: PathBuf,
        decoder: PathBuf,
        tokens: PathBuf,
        #[serde(default = "english")]
        language: String,
        #[serde(default = "default_threads")]
        threads: i32,
    },
    MoonshineV2 {
        encoder: PathBuf,
        merged_decoder: PathBuf,
        tokens: PathBuf,
        #[serde(default = "default_threads")]
        threads: i32,
    },
    NemoTransducer {
        encoder: PathBuf,
        decoder: PathBuf,
        joiner: PathBuf,
        tokens: PathBuf,
        #[serde(default = "default_threads")]
        threads: i32,
    },
}

fn english() -> String {
    "en".to_owned()
}

#[derive(Debug, Deserialize)]
#[serde(tag = "family", rename_all = "kebab-case")]
enum TtsSection {
    Supertonic {
        duration_predictor: PathBuf,
        text_encoder: PathBuf,
        vector_estimator: PathBuf,
        vocoder: PathBuf,
        tts_json: PathBuf,
        unicode_indexer: PathBuf,
        voice_style: PathBuf,
        #[serde(default = "default_threads")]
        threads: i32,
        #[serde(default = "default_speed")]
        speed: f32,
        #[serde(default)]
        speaker_id: i32,
    },
    Kitten {
        model: PathBuf,
        voices: PathBuf,
        tokens: PathBuf,
        data_dir: PathBuf,
        #[serde(default = "default_threads")]
        threads: i32,
        #[serde(default = "default_speed")]
        speed: f32,
        #[serde(default)]
        speaker_id: i32,
    },
    Kokoro {
        model: PathBuf,
        voices: PathBuf,
        tokens: PathBuf,
        data_dir: PathBuf,
        lexicon: Option<PathBuf>,
        lang: Option<String>,
        #[serde(default = "default_threads")]
        threads: i32,
        #[serde(default = "default_speed")]
        speed: f32,
        #[serde(default)]
        speaker_id: i32,
    },
}

impl SttSection {
    fn config(self) -> SherpaSttConfig {
        match self {
            Self::Whisper {
                encoder,
                decoder,
                tokens,
                language,
                threads,
            } => SherpaSttConfig::new(
                SherpaSttModel::Whisper {
                    encoder,
                    decoder,
                    tokens,
                    language,
                },
                threads,
            ),
            Self::MoonshineV2 {
                encoder,
                merged_decoder,
                tokens,
                threads,
            } => SherpaSttConfig::new(
                SherpaSttModel::MoonshineV2 {
                    encoder,
                    merged_decoder,
                    tokens,
                },
                threads,
            ),
            Self::NemoTransducer {
                encoder,
                decoder,
                joiner,
                tokens,
                threads,
            } => SherpaSttConfig::new(
                SherpaSttModel::NemoTransducer {
                    encoder,
                    decoder,
                    joiner,
                    tokens,
                },
                threads,
            ),
        }
    }
}

impl TtsSection {
    fn config(self) -> SherpaTtsConfig {
        let (model, threads, speed, speaker_id) = match self {
            Self::Supertonic {
                duration_predictor,
                text_encoder,
                vector_estimator,
                vocoder,
                tts_json,
                unicode_indexer,
                voice_style,
                threads,
                speed,
                speaker_id,
            } => (
                SherpaTtsModel::Supertonic {
                    duration_predictor,
                    text_encoder,
                    vector_estimator,
                    vocoder,
                    tts_json,
                    unicode_indexer,
                    voice_style,
                },
                threads,
                speed,
                speaker_id,
            ),
            Self::Kitten {
                model,
                voices,
                tokens,
                data_dir,
                threads,
                speed,
                speaker_id,
            } => (
                SherpaTtsModel::Kitten {
                    model,
                    voices,
                    tokens,
                    data_dir,
                },
                threads,
                speed,
                speaker_id,
            ),
            Self::Kokoro {
                model,
                voices,
                tokens,
                data_dir,
                lexicon,
                lang,
                threads,
                speed,
                speaker_id,
            } => (
                SherpaTtsModel::Kokoro {
                    model,
                    voices,
                    tokens,
                    data_dir,
                    lexicon,
                    lang,
                },
                threads,
                speed,
                speaker_id,
            ),
        };
        let mut config = SherpaTtsConfig::new(model, threads);
        config.speed = speed;
        config.speaker_id = speaker_id;
        config
    }
}

/// Reads the engines file and prepares what the run needs. The recogniser and
/// the synthesiser load on their worker threads; the VAD loads here.
pub fn load(
    need: Need,
    file: Option<&Path>,
) -> Result<(Option<ListenParts>, Option<TtsLoad>), Problem> {
    let path = file.ok_or_else(|| {
        Problem(
            "speech models are named by a TOML file: pass --engines <file> (see src/sherpa.rs for its form)"
                .to_owned(),
        )
    })?;
    let text = std::fs::read_to_string(path)
        .map_err(|error| Problem(format!("cannot read {}: {error}", path.display())))?;
    let file: File = toml::from_str(&text).map_err(|error| {
        Problem(format!(
            "{} is not valid: {}",
            path.display(),
            error.message()
        ))
    })?;

    let listen = if need.listen {
        let vad = file
            .vad
            .ok_or_else(|| Problem("the engines file has no [vad] section".to_owned()))?;
        let stt = file
            .stt
            .ok_or_else(|| Problem("the engines file has no [stt] section".to_owned()))?
            .config();
        let vad_config = SherpaVadConfig::new(vad.model);
        let vad = SherpaVad::load(&vad_config)
            .map_err(|error| Problem(format!("the VAD did not load: {error}")))?;
        Some(ListenParts {
            vad: Box::new(vad),
            stt: Box::new(move || {
                SherpaStt::load(&stt).map(|engine| Box::new(engine) as Box<dyn SttEngine>)
            }),
        })
    } else {
        None
    };
    let tts = if need.speak {
        let config = file
            .tts
            .ok_or_else(|| Problem("the engines file has no [tts] section".to_owned()))?
            .config();
        Some(Box::new(move || {
            SherpaTts::load(&config).map(|engine| Box::new(engine) as Box<dyn TtsEngine>)
        }) as TtsLoad)
    } else {
        None
    };
    Ok((listen, tts))
}
