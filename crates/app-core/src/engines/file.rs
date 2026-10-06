//! The engines file: which model files the speech engines load.
//!
//! ```toml
//! [vad]
//! model = "silero_vad.onnx"
//!
//! [stt]
//! family = "whisper"            # or "moonshine-v2", or "nemo-transducer"
//! encoder = "whisper/encoder.onnx"
//! decoder = "whisper/decoder.onnx"
//! tokens = "whisper/tokens.txt"
//! language = "en"
//! threads = 4
//!
//! [tts]                         # only needed when the tutor speaks
//! family = "kokoro"             # or "kitten", or "supertonic"
//! model = "kokoro/model.onnx"
//! ```
//!
//! A path that is not absolute is resolved against the folder the caller names
//! (the models folder of the server, the working directory for `tutor-cli`).
//! Reading the file needs no native library, so what it says can be checked in
//! every build; loading the models needs the `sherpa` feature.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use speech::{
    SherpaConfigError, SherpaSttConfig, SherpaSttModel, SherpaTtsConfig, SherpaTtsModel,
    SherpaVadConfig,
};

fn default_threads() -> i32 {
    4
}

fn default_speed() -> f32 {
    1.0
}

fn english() -> String {
    "en".to_owned()
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

/// Resolves a path against `base` when it is relative.
struct Paths<'a> {
    base: Option<&'a Path>,
}

impl Paths<'_> {
    fn of(&self, path: PathBuf) -> PathBuf {
        match self.base {
            Some(base) if path.is_relative() => base.join(path),
            _ => path,
        }
    }
}

impl SttSection {
    fn config(self, paths: &Paths<'_>) -> SherpaSttConfig {
        match self {
            Self::Whisper {
                encoder,
                decoder,
                tokens,
                language,
                threads,
            } => SherpaSttConfig::new(
                SherpaSttModel::Whisper {
                    encoder: paths.of(encoder),
                    decoder: paths.of(decoder),
                    tokens: paths.of(tokens),
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
                    encoder: paths.of(encoder),
                    merged_decoder: paths.of(merged_decoder),
                    tokens: paths.of(tokens),
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
                    encoder: paths.of(encoder),
                    decoder: paths.of(decoder),
                    joiner: paths.of(joiner),
                    tokens: paths.of(tokens),
                },
                threads,
            ),
        }
    }
}

impl TtsSection {
    fn config(self, paths: &Paths<'_>) -> SherpaTtsConfig {
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
                    duration_predictor: paths.of(duration_predictor),
                    text_encoder: paths.of(text_encoder),
                    vector_estimator: paths.of(vector_estimator),
                    vocoder: paths.of(vocoder),
                    tts_json: paths.of(tts_json),
                    unicode_indexer: paths.of(unicode_indexer),
                    voice_style: paths.of(voice_style),
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
                    model: paths.of(model),
                    voices: paths.of(voices),
                    tokens: paths.of(tokens),
                    data_dir: paths.of(data_dir),
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
                    model: paths.of(model),
                    voices: paths.of(voices),
                    tokens: paths.of(tokens),
                    data_dir: paths.of(data_dir),
                    lexicon: lexicon.map(|path| paths.of(path)),
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

/// What the engines file names. A section the file does not have is `None`.
#[derive(Debug, Clone)]
pub struct EnginesFile {
    pub vad: Option<SherpaVadConfig>,
    pub stt: Option<SherpaSttConfig>,
    pub tts: Option<SherpaTtsConfig>,
}

/// Reads the engines file text. The error is a sentence without a path.
pub fn parse(text: &str, base: Option<&Path>) -> Result<EnginesFile, String> {
    let file: File = toml::from_str(text)
        .map_err(|error| format!("the engines file is not valid: {}", error.message()))?;
    let paths = Paths { base };
    Ok(EnginesFile {
        vad: file
            .vad
            .map(|vad| SherpaVadConfig::new(paths.of(vad.model))),
        stt: file.stt.map(|stt| stt.config(&paths)),
        tts: file.tts.map(|tts| tts.config(&paths)),
    })
}

/// A configuration error as a sentence for the learner: it names the role and
/// what is missing, never the path.
pub fn sentence(error: &SherpaConfigError) -> String {
    match error {
        SherpaConfigError::Missing { role, expected, .. } => {
            format!("the {role} {expected} is missing")
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHISPER: &str = r#"
        [vad]
        model = "silero.onnx"
        [stt]
        family = "whisper"
        encoder = "w/enc.onnx"
        decoder = "w/dec.onnx"
        tokens = "w/tokens.txt"
    "#;

    #[test]
    fn relative_paths_are_joined_to_the_base_and_absolute_ones_are_kept() {
        let base = Path::new("/models");
        let parsed = parse(WHISPER, Some(base)).expect("parses");
        assert_eq!(
            parsed.vad.expect("vad").model,
            Path::new("/models/silero.onnx")
        );
        let stt = parsed.stt.expect("stt");
        assert_eq!(stt.num_threads, 4);
        assert!(parsed.tts.is_none());
        let absolute = parse("[vad]\nmodel = \"/elsewhere/v.onnx\"", Some(base)).expect("parses");
        assert_eq!(
            absolute.vad.expect("vad").model,
            Path::new("/elsewhere/v.onnx")
        );
        let plain = parse(WHISPER, None).expect("parses");
        assert_eq!(plain.vad.expect("vad").model, Path::new("silero.onnx"));
    }

    #[test]
    fn a_bad_file_says_so_without_a_path() {
        let error = parse("[stt]\nfamily = \"nope\"", None).expect_err("refused");
        assert!(
            error.starts_with("the engines file is not valid"),
            "{error}"
        );
        let error = parse("not toml at all [", None).expect_err("refused");
        assert!(
            error.starts_with("the engines file is not valid"),
            "{error}"
        );
    }

    #[test]
    fn a_missing_model_is_reported_by_role_only() {
        let parsed = parse(WHISPER, Some(Path::new("/definitely/not/here"))).expect("parses");
        let problem = parsed.vad.expect("vad").validate().expect_err("missing");
        let text = sentence(&problem);
        assert_eq!(text, "the VAD model file is missing");
        assert!(!text.contains("definitely"));
    }
}
