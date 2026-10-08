//! Which files a sherpa-onnx model needs, and the checks that run before the
//! native library is asked to load them.
//!
//! These types compile without the `sherpa` feature, so the checks are tested
//! everywhere. The adapters that use them are in `sherpa.rs` and need the
//! native library.
//!
//! The field names follow the `sherpa-onnx` 1.13.8 crate's own configuration
//! structs. Which files a given downloaded model really contains, and what they
//! are called, is read from the model's own folder by the owner when the
//! manifest entry is filled in; nothing here assumes a file name.

use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::EngineInfo;

/// Silero VAD takes 512 samples (32 ms at 16 kHz) per call.
pub const SHERPA_VAD_FRAME_SAMPLES: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SherpaConfigError {
    #[error("the {role} path {path} is not a {expected}")]
    Missing {
        role: &'static str,
        path: PathBuf,
        expected: &'static str,
    },
    #[error(
        "the {role} path cannot be passed to the native library: it is not UTF-8 or holds a NUL"
    )]
    BadPath { role: &'static str },
    #[error("num_threads must be at least 1")]
    Threads,
    #[error("{0}")]
    Value(&'static str),
}

#[derive(Debug, Clone, Copy)]
enum Kind {
    File,
    Dir,
}

struct Entry<'a> {
    role: &'static str,
    path: &'a Path,
    kind: Kind,
}

fn file<'a>(role: &'static str, path: &'a Path) -> Entry<'a> {
    Entry {
        role,
        path,
        kind: Kind::File,
    }
}

fn dir<'a>(role: &'static str, path: &'a Path) -> Entry<'a> {
    Entry {
        role,
        path,
        kind: Kind::Dir,
    }
}

/// The native wrapper turns every path into a C string and panics on a NUL, so
/// the check has to happen first. A missing file is reported here with its role
/// instead of as a null pointer from the library.
fn check(entries: &[Entry<'_>]) -> Result<(), SherpaConfigError> {
    for entry in entries {
        match entry.path.to_str() {
            Some(text) if !text.contains('\0') && !text.is_empty() => {}
            _ => return Err(SherpaConfigError::BadPath { role: entry.role }),
        }
        let present = match entry.kind {
            Kind::File => entry.path.is_file(),
            Kind::Dir => entry.path.is_dir(),
        };
        if !present {
            return Err(SherpaConfigError::Missing {
                role: entry.role,
                path: entry.path.to_path_buf(),
                expected: match entry.kind {
                    Kind::File => "file",
                    Kind::Dir => "folder",
                },
            });
        }
    }
    Ok(())
}

fn threads(n: i32) -> Result<(), SherpaConfigError> {
    if n < 1 {
        return Err(SherpaConfigError::Threads);
    }
    Ok(())
}

/// Silero VAD through sherpa-onnx.
///
/// The sherpa-onnx VAD reports whether speech is present, not a probability
/// per frame. The adapter maps that to 0.0 or 1.0, so the hysteresis of the
/// endpointer has nothing to work with and the sherpa detector's own minimum
/// durations add to the endpointer's. The defaults are starting points, not
/// measured values; F2 decides.
#[derive(Debug, Clone)]
pub struct SherpaVadConfig {
    pub model: PathBuf,
    pub threshold: f32,
    /// Silence the sherpa detector needs before it reports the end of speech.
    pub min_silence_s: f32,
    pub min_speech_s: f32,
    pub num_threads: i32,
}

impl SherpaVadConfig {
    pub fn new(model: impl Into<PathBuf>) -> Self {
        Self {
            model: model.into(),
            threshold: 0.5,
            min_silence_s: 0.1,
            min_speech_s: 0.1,
            num_threads: 1,
        }
    }

    pub fn validate(&self) -> Result<(), SherpaConfigError> {
        check(&[file("VAD model", &self.model)])?;
        threads(self.num_threads)?;
        if !(0.0..=1.0).contains(&self.threshold) {
            return Err(SherpaConfigError::Value("the VAD threshold must be 0 to 1"));
        }
        if self.min_silence_s < 0.0 || self.min_speech_s < 0.0 {
            return Err(SherpaConfigError::Value(
                "VAD durations must not be negative",
            ));
        }
        Ok(())
    }
}

/// The recogniser families the project may use. All three are whole-utterance
/// ("offline") models: `feed` collects audio and `finish` decodes it.
#[derive(Debug, Clone)]
pub enum SherpaSttModel {
    Whisper {
        encoder: PathBuf,
        decoder: PathBuf,
        tokens: PathBuf,
        /// For example `en`.
        language: String,
    },
    /// Moonshine v2: an encoder and a merged decoder.
    MoonshineV2 {
        encoder: PathBuf,
        merged_decoder: PathBuf,
        tokens: PathBuf,
    },
    /// A NeMo transducer such as Parakeet TDT.
    NemoTransducer {
        encoder: PathBuf,
        decoder: PathBuf,
        joiner: PathBuf,
        tokens: PathBuf,
    },
}

impl SherpaSttModel {
    /// Short name used in `EngineInfo`.
    pub fn family(&self) -> &'static str {
        match self {
            Self::Whisper { .. } => "whisper",
            Self::MoonshineV2 { .. } => "moonshine-v2",
            Self::NemoTransducer { .. } => "nemo-transducer",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SherpaSttConfig {
    pub model: SherpaSttModel,
    pub num_threads: i32,
    /// Combined SHA-256 of the model files, from the model manager's record.
    pub model_checksum: Option<String>,
}

impl SherpaSttConfig {
    pub fn new(model: SherpaSttModel, num_threads: i32) -> Self {
        Self {
            model,
            num_threads,
            model_checksum: None,
        }
    }

    pub fn validate(&self) -> Result<(), SherpaConfigError> {
        threads(self.num_threads)?;
        match &self.model {
            SherpaSttModel::Whisper {
                encoder,
                decoder,
                tokens,
                language,
            } => {
                if language.is_empty() || language.contains('\0') {
                    return Err(SherpaConfigError::Value(
                        "the Whisper language must not be empty",
                    ));
                }
                check(&[
                    file("Whisper encoder", encoder),
                    file("Whisper decoder", decoder),
                    file("tokens", tokens),
                ])
            }
            SherpaSttModel::MoonshineV2 {
                encoder,
                merged_decoder,
                tokens,
            } => check(&[
                file("Moonshine encoder", encoder),
                file("Moonshine merged decoder", merged_decoder),
                file("tokens", tokens),
            ]),
            SherpaSttModel::NemoTransducer {
                encoder,
                decoder,
                joiner,
                tokens,
            } => check(&[
                file("transducer encoder", encoder),
                file("transducer decoder", decoder),
                file("transducer joiner", joiner),
                file("tokens", tokens),
            ]),
        }
    }

    #[cfg_attr(not(feature = "sherpa"), allow(dead_code))]
    pub(crate) fn engine_info(&self, native_version: &str) -> EngineInfo {
        let info = EngineInfo::new(
            format!("sherpa-onnx/{}", self.model.family()),
            native_version.to_owned(),
        );
        match &self.model_checksum {
            Some(checksum) => info.with_model_checksum(checksum.clone()),
            None => info,
        }
    }
}

/// The synthesiser families the project may use.
#[derive(Debug, Clone)]
pub enum SherpaTtsModel {
    Supertonic {
        duration_predictor: PathBuf,
        text_encoder: PathBuf,
        vector_estimator: PathBuf,
        vocoder: PathBuf,
        tts_json: PathBuf,
        unicode_indexer: PathBuf,
        voice_style: PathBuf,
    },
    Kitten {
        model: PathBuf,
        voices: PathBuf,
        tokens: PathBuf,
        data_dir: PathBuf,
    },
    Kokoro {
        model: PathBuf,
        voices: PathBuf,
        tokens: PathBuf,
        data_dir: PathBuf,
        lexicon: Option<PathBuf>,
        lang: Option<String>,
    },
}

impl SherpaTtsModel {
    pub fn family(&self) -> &'static str {
        match self {
            Self::Supertonic { .. } => "supertonic",
            Self::Kitten { .. } => "kitten",
            Self::Kokoro { .. } => "kokoro",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SherpaTtsConfig {
    pub model: SherpaTtsModel,
    pub num_threads: i32,
    /// Speaking speed, 1.0 is the model's own pace.
    pub speed: f32,
    pub speaker_id: i32,
    pub model_checksum: Option<String>,
}

impl SherpaTtsConfig {
    pub fn new(model: SherpaTtsModel, num_threads: i32) -> Self {
        Self {
            model,
            num_threads,
            speed: 1.0,
            speaker_id: 0,
            model_checksum: None,
        }
    }

    pub fn validate(&self) -> Result<(), SherpaConfigError> {
        threads(self.num_threads)?;
        if !(self.speed > 0.0 && self.speed.is_finite()) {
            return Err(SherpaConfigError::Value("the speed must be above 0"));
        }
        if self.speaker_id < 0 {
            return Err(SherpaConfigError::Value(
                "the speaker id must not be negative",
            ));
        }
        match &self.model {
            SherpaTtsModel::Supertonic {
                duration_predictor,
                text_encoder,
                vector_estimator,
                vocoder,
                tts_json,
                unicode_indexer,
                voice_style,
            } => check(&[
                file("Supertonic duration predictor", duration_predictor),
                file("Supertonic text encoder", text_encoder),
                file("Supertonic vector estimator", vector_estimator),
                file("Supertonic vocoder", vocoder),
                file("Supertonic tts.json", tts_json),
                file("Supertonic unicode indexer", unicode_indexer),
                file("Supertonic voice style", voice_style),
            ]),
            SherpaTtsModel::Kitten {
                model,
                voices,
                tokens,
                data_dir,
            } => check(&[
                file("Kitten model", model),
                file("Kitten voices", voices),
                file("tokens", tokens),
                dir("data folder", data_dir),
            ]),
            SherpaTtsModel::Kokoro {
                model,
                voices,
                tokens,
                data_dir,
                lexicon,
                lang,
            } => {
                if lang.as_deref().is_some_and(|l| l.contains('\0')) {
                    return Err(SherpaConfigError::Value("the language holds a NUL"));
                }
                let mut entries = vec![
                    file("Kokoro model", model),
                    file("Kokoro voices", voices),
                    file("tokens", tokens),
                    dir("data folder", data_dir),
                ];
                if let Some(lexicon) = lexicon {
                    entries.push(file("lexicon", lexicon));
                }
                check(&entries)
            }
        }
    }

    #[cfg_attr(not(feature = "sherpa"), allow(dead_code))]
    pub(crate) fn engine_info(&self, native_version: &str) -> EngineInfo {
        let info = EngineInfo::new(
            format!("sherpa-onnx/{}", self.model.family()),
            native_version.to_owned(),
        );
        match &self.model_checksum {
            Some(checksum) => info.with_model_checksum(checksum.clone()),
            None => info,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn touch(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, b"x").expect("write");
        path
    }

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lumingo-speech-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    #[test]
    fn a_missing_file_is_named_by_its_role() {
        let dir = temp();
        let config = SherpaVadConfig::new(dir.join("silero.onnx"));
        assert!(matches!(
            config.validate(),
            Err(SherpaConfigError::Missing {
                role: "VAD model",
                ..
            })
        ));
        touch(&dir, "silero.onnx");
        assert_eq!(config.validate(), Ok(()));
        fs::remove_dir_all(dir).expect("cleanup");
    }

    #[test]
    fn a_nul_or_empty_path_is_refused_before_the_native_library_can_panic() {
        let bad = SherpaVadConfig::new("a\0b");
        assert_eq!(
            bad.validate(),
            Err(SherpaConfigError::BadPath { role: "VAD model" })
        );
        let empty = SherpaVadConfig::new("");
        assert!(matches!(
            empty.validate(),
            Err(SherpaConfigError::BadPath { .. })
        ));
    }

    #[test]
    fn vad_values_are_checked() {
        let dir = temp();
        let model = touch(&dir, "m.onnx");
        let mut c = SherpaVadConfig::new(&model);
        c.threshold = 1.5;
        assert!(matches!(c.validate(), Err(SherpaConfigError::Value(_))));
        let mut c = SherpaVadConfig::new(&model);
        c.num_threads = 0;
        assert_eq!(c.validate(), Err(SherpaConfigError::Threads));
        let mut c = SherpaVadConfig::new(&model);
        c.min_silence_s = -1.0;
        assert!(matches!(c.validate(), Err(SherpaConfigError::Value(_))));
        fs::remove_dir_all(dir).expect("cleanup");
    }

    #[test]
    fn every_stt_family_checks_each_of_its_files() {
        let dir = temp();
        let a = touch(&dir, "a");
        let b = touch(&dir, "b");
        let c = touch(&dir, "c");
        let t = touch(&dir, "tokens");
        let gone = dir.join("gone");

        let whisper = |decoder: &Path, language: &str| {
            SherpaSttConfig::new(
                SherpaSttModel::Whisper {
                    encoder: a.clone(),
                    decoder: decoder.to_path_buf(),
                    tokens: t.clone(),
                    language: language.to_owned(),
                },
                2,
            )
        };
        assert_eq!(whisper(&b, "en").validate(), Ok(()));
        assert!(matches!(
            whisper(&gone, "en").validate(),
            Err(SherpaConfigError::Missing {
                role: "Whisper decoder",
                ..
            })
        ));
        assert!(matches!(
            whisper(&b, "").validate(),
            Err(SherpaConfigError::Value(_))
        ));

        let moonshine = |decoder: &Path| {
            SherpaSttConfig::new(
                SherpaSttModel::MoonshineV2 {
                    encoder: a.clone(),
                    merged_decoder: decoder.to_path_buf(),
                    tokens: t.clone(),
                },
                2,
            )
        };
        assert_eq!(moonshine(&b).validate(), Ok(()));
        assert!(moonshine(&gone).validate().is_err());

        let transducer = |joiner: &Path| {
            SherpaSttConfig::new(
                SherpaSttModel::NemoTransducer {
                    encoder: a.clone(),
                    decoder: b.clone(),
                    joiner: joiner.to_path_buf(),
                    tokens: t.clone(),
                },
                4,
            )
        };
        assert_eq!(transducer(&c).validate(), Ok(()));
        assert!(matches!(
            transducer(&gone).validate(),
            Err(SherpaConfigError::Missing {
                role: "transducer joiner",
                ..
            })
        ));
        fs::remove_dir_all(dir).expect("cleanup");
    }

    #[test]
    fn tts_configs_check_files_folders_and_numbers() {
        let dir = temp();
        let m = touch(&dir, "model.onnx");
        let v = touch(&dir, "voices.bin");
        let t = touch(&dir, "tokens.txt");
        let data = dir.join("data");
        fs::create_dir_all(&data).expect("mkdir");

        let kitten = |data_dir: &Path| {
            SherpaTtsConfig::new(
                SherpaTtsModel::Kitten {
                    model: m.clone(),
                    voices: v.clone(),
                    tokens: t.clone(),
                    data_dir: data_dir.to_path_buf(),
                },
                2,
            )
        };
        assert_eq!(kitten(&data).validate(), Ok(()));
        // A file where a folder is expected is refused.
        assert!(matches!(
            kitten(&m).validate(),
            Err(SherpaConfigError::Missing {
                expected: "folder",
                ..
            })
        ));

        let mut config = kitten(&data);
        config.speed = 0.0;
        assert!(matches!(
            config.validate(),
            Err(SherpaConfigError::Value(_))
        ));
        config.speed = f32::NAN;
        assert!(config.validate().is_err());
        config.speed = 1.0;
        config.speaker_id = -1;
        assert!(config.validate().is_err());

        let kokoro = SherpaTtsConfig::new(
            SherpaTtsModel::Kokoro {
                model: m.clone(),
                voices: v.clone(),
                tokens: t.clone(),
                data_dir: data.clone(),
                lexicon: Some(dir.join("lexicon-missing.txt")),
                lang: Some("en-us".into()),
            },
            2,
        );
        assert!(matches!(
            kokoro.validate(),
            Err(SherpaConfigError::Missing {
                role: "lexicon",
                ..
            })
        ));

        let supertonic = SherpaTtsConfig::new(
            SherpaTtsModel::Supertonic {
                duration_predictor: m.clone(),
                text_encoder: m.clone(),
                vector_estimator: m.clone(),
                vocoder: m.clone(),
                tts_json: m.clone(),
                unicode_indexer: m.clone(),
                voice_style: dir.join("missing.json"),
            },
            2,
        );
        assert!(matches!(
            supertonic.validate(),
            Err(SherpaConfigError::Missing {
                role: "Supertonic voice style",
                ..
            })
        ));
        fs::remove_dir_all(dir).expect("cleanup");
    }

    #[test]
    fn engine_info_carries_the_family_and_the_model_checksum() {
        let mut config = SherpaSttConfig::new(
            SherpaSttModel::MoonshineV2 {
                encoder: "e".into(),
                merged_decoder: "d".into(),
                tokens: "t".into(),
            },
            1,
        );
        config.model_checksum = Some("abc".into());
        let info = config.engine_info("1.13.8");
        assert_eq!(info.id, "sherpa-onnx/moonshine-v2");
        assert_eq!(info.version, "1.13.8");
        assert_eq!(info.model_checksum.as_deref(), Some("abc"));
    }
}
