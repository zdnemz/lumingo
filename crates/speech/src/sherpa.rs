//! sherpa-onnx adapters for the VAD, STT and TTS traits.
//!
//! UNVERIFIED. This module is compiled only with the `sherpa` feature, and the
//! build container cannot download the native library, so it has been
//! compile-checked against the real `sherpa-onnx` 1.13.8 API but never linked
//! or run. Nothing here has loaded a model. The owner runs it on Windows with
//! the real model files.
//!
//! The wrapper crate calls `unwrap` on every path it converts to a C string and
//! on the text of a synthesis call, so a NUL byte would panic inside it. The
//! configuration is validated first ([`crate::sherpa_config`]) and NULs are
//! removed from text here.

use sherpa_onnx::{
    GenerationConfig, OfflineModelConfig, OfflineMoonshineModelConfig, OfflineRecognizer,
    OfflineRecognizerConfig, OfflineTransducerModelConfig, OfflineTts, OfflineTtsConfig,
    OfflineTtsKittenModelConfig, OfflineTtsKokoroModelConfig, OfflineTtsModelConfig,
    OfflineTtsSupertonicModelConfig, OfflineWhisperModelConfig, SileroVadModelConfig,
    VadModelConfig, VoiceActivityDetector,
};

use crate::sherpa_config::{
    SHERPA_VAD_FRAME_SAMPLES, SherpaConfigError, SherpaSttConfig, SherpaSttModel, SherpaTtsConfig,
    SherpaTtsModel, SherpaVadConfig,
};
use crate::{
    CancelFlag, EngineInfo, PcmChunk, SPEECH_SAMPLE_RATE, SttEngine, SttError, Transcript,
    TtsEngine, TtsError, Vad, VadError,
};

/// Version of the `sherpa-onnx` crate this adapter was written against. The
/// native library the build links is the release with the same number.
pub const SHERPA_ONNX_CRATE_VERSION: &str = "1.13.8";

/// Longest audio the STT adapter accepts: the 30 s utterance limit of the
/// endpointer plus a margin.
const MAX_STT_SAMPLES: usize = SPEECH_SAMPLE_RATE as usize * 35;

fn path_string(path: &std::path::Path) -> Option<String> {
    path.to_str().map(str::to_owned)
}

/// Silero VAD through sherpa-onnx.
///
/// The wrapper exposes `detected()`, not a probability, so `push_frame`
/// returns 0.0 or 1.0. See [`SherpaVadConfig`].
pub struct SherpaVad {
    detector: VoiceActivityDetector,
}

impl SherpaVad {
    pub fn load(config: &SherpaVadConfig) -> Result<Self, VadError> {
        config.validate().map_err(vad_unavailable)?;
        let model_config = VadModelConfig {
            silero_vad: SileroVadModelConfig {
                model: path_string(&config.model),
                threshold: config.threshold,
                min_silence_duration: config.min_silence_s,
                min_speech_duration: config.min_speech_s,
                window_size: SHERPA_VAD_FRAME_SAMPLES as i32,
                // The endpointer forces an end at 30 s, so the detector may too.
                max_speech_duration: 30.0,
            },
            sample_rate: SPEECH_SAMPLE_RATE as i32,
            num_threads: config.num_threads,
            provider: Some("cpu".to_owned()),
            debug: false,
            ..VadModelConfig::default()
        };
        let detector = VoiceActivityDetector::create(&model_config, 30.0).ok_or_else(|| {
            VadError::Engine("sherpa-onnx could not create the voice activity detector".to_owned())
        })?;
        Ok(Self { detector })
    }
}

fn vad_unavailable(error: SherpaConfigError) -> VadError {
    VadError::Unavailable {
        reason: error.to_string(),
    }
}

impl Vad for SherpaVad {
    fn push_frame(&mut self, frame: &[f32]) -> Result<f32, VadError> {
        if frame.len() != SHERPA_VAD_FRAME_SAMPLES {
            return Err(VadError::FrameSize {
                expected: SHERPA_VAD_FRAME_SAMPLES,
                got: frame.len(),
            });
        }
        self.detector.accept_waveform(frame);
        let probability = if self.detector.detected() { 1.0 } else { 0.0 };
        // The finished segments are not used: the endpointer cuts the audio.
        // Draining keeps the detector's queue from growing for the whole session.
        while !self.detector.is_empty() {
            self.detector.pop();
        }
        Ok(probability)
    }

    fn reset(&mut self) {
        self.detector.reset();
    }
}

/// A whole-utterance recogniser: `feed` collects audio, `finish` decodes it.
///
/// The decode is one native call that cannot be interrupted, so the cancel flag
/// is checked before it and after it. A cancelled result is dropped.
pub struct SherpaStt {
    recognizer: OfflineRecognizer,
    samples: Vec<f32>,
    info: EngineInfo,
}

impl SherpaStt {
    pub fn load(config: &SherpaSttConfig) -> Result<Self, SttError> {
        config.validate().map_err(|e| SttError::Unavailable {
            reason: e.to_string(),
        })?;
        let mut model_config = OfflineModelConfig {
            num_threads: config.num_threads,
            provider: Some("cpu".to_owned()),
            debug: false,
            ..OfflineModelConfig::default()
        };
        match &config.model {
            SherpaSttModel::Whisper {
                encoder,
                decoder,
                tokens,
                language,
            } => {
                model_config.whisper = OfflineWhisperModelConfig {
                    encoder: path_string(encoder),
                    decoder: path_string(decoder),
                    language: Some(language.clone()),
                    task: Some("transcribe".to_owned()),
                    ..OfflineWhisperModelConfig::default()
                };
                model_config.tokens = path_string(tokens);
            }
            SherpaSttModel::MoonshineV2 {
                encoder,
                merged_decoder,
                tokens,
            } => {
                model_config.moonshine = OfflineMoonshineModelConfig {
                    encoder: path_string(encoder),
                    merged_decoder: path_string(merged_decoder),
                    ..OfflineMoonshineModelConfig::default()
                };
                model_config.tokens = path_string(tokens);
            }
            SherpaSttModel::NemoTransducer {
                encoder,
                decoder,
                joiner,
                tokens,
            } => {
                model_config.transducer = OfflineTransducerModelConfig {
                    encoder: path_string(encoder),
                    decoder: path_string(decoder),
                    joiner: path_string(joiner),
                };
                model_config.tokens = path_string(tokens);
                model_config.model_type = Some("nemo_transducer".to_owned());
            }
        }
        let recognizer_config = OfflineRecognizerConfig {
            model_config,
            decoding_method: Some("greedy_search".to_owned()),
            ..OfflineRecognizerConfig::default()
        };
        let recognizer = OfflineRecognizer::create(&recognizer_config).ok_or_else(|| {
            SttError::Engine(
                "sherpa-onnx could not create the recogniser from these model files".to_owned(),
            )
        })?;
        Ok(Self {
            recognizer,
            samples: Vec::new(),
            info: config.engine_info(SHERPA_ONNX_CRATE_VERSION),
        })
    }
}

impl SttEngine for SherpaStt {
    fn start(&mut self) -> Result<(), SttError> {
        self.samples.clear();
        Ok(())
    }

    fn feed(&mut self, samples: &[f32]) -> Result<(), SttError> {
        if self.samples.len() + samples.len() > MAX_STT_SAMPLES {
            return Err(SttError::Engine(
                "the utterance is longer than the 35 seconds this engine accepts".to_owned(),
            ));
        }
        self.samples.extend_from_slice(samples);
        Ok(())
    }

    fn finish(&mut self, cancel: &CancelFlag) -> Result<Transcript, SttError> {
        let samples = std::mem::take(&mut self.samples);
        if cancel.is_cancelled() {
            return Err(SttError::Cancelled);
        }
        let duration_ms =
            u32::try_from(samples.len() as u64 * 1000 / u64::from(SPEECH_SAMPLE_RATE))
                .unwrap_or(u32::MAX);
        if samples.is_empty() {
            return Ok(Transcript::default());
        }
        let stream = self.recognizer.create_stream();
        stream.accept_waveform(SPEECH_SAMPLE_RATE as i32, &samples);
        self.recognizer.decode(&stream);
        let result = stream.get_result();
        if cancel.is_cancelled() {
            return Err(SttError::Cancelled);
        }
        let result = result
            .ok_or_else(|| SttError::Engine("the recogniser returned no result".to_owned()))?;
        Ok(Transcript {
            text: result.text.trim().to_owned(),
            // Word timings are not read: the wrapper gives token times, and
            // grouping them into words is left for the pronunciation work.
            words: Vec::new(),
            duration_ms,
        })
    }

    fn info(&self) -> EngineInfo {
        self.info.clone()
    }
}

/// A synthesiser. Generation stops early when the cancel flag is set, through
/// the progress callback the wrapper offers.
pub struct SherpaTts {
    tts: OfflineTts,
    speed: f32,
    speaker_id: i32,
    info: EngineInfo,
}

impl SherpaTts {
    pub fn load(config: &SherpaTtsConfig) -> Result<Self, TtsError> {
        config.validate().map_err(|e| TtsError::Unavailable {
            reason: e.to_string(),
        })?;
        let mut model = OfflineTtsModelConfig {
            num_threads: config.num_threads,
            provider: Some("cpu".to_owned()),
            debug: false,
            ..OfflineTtsModelConfig::default()
        };
        match &config.model {
            SherpaTtsModel::Supertonic {
                duration_predictor,
                text_encoder,
                vector_estimator,
                vocoder,
                tts_json,
                unicode_indexer,
                voice_style,
            } => {
                model.supertonic = OfflineTtsSupertonicModelConfig {
                    duration_predictor: path_string(duration_predictor),
                    text_encoder: path_string(text_encoder),
                    vector_estimator: path_string(vector_estimator),
                    vocoder: path_string(vocoder),
                    tts_json: path_string(tts_json),
                    unicode_indexer: path_string(unicode_indexer),
                    voice_style: path_string(voice_style),
                };
            }
            SherpaTtsModel::Kitten {
                model: model_file,
                voices,
                tokens,
                data_dir,
            } => {
                model.kitten = OfflineTtsKittenModelConfig {
                    model: path_string(model_file),
                    voices: path_string(voices),
                    tokens: path_string(tokens),
                    data_dir: path_string(data_dir),
                    ..OfflineTtsKittenModelConfig::default()
                };
            }
            SherpaTtsModel::Kokoro {
                model: model_file,
                voices,
                tokens,
                data_dir,
                lexicon,
                lang,
            } => {
                model.kokoro = OfflineTtsKokoroModelConfig {
                    model: path_string(model_file),
                    voices: path_string(voices),
                    tokens: path_string(tokens),
                    data_dir: path_string(data_dir),
                    lexicon: lexicon.as_deref().and_then(path_string),
                    lang: lang.clone(),
                    ..OfflineTtsKokoroModelConfig::default()
                };
            }
        }
        let tts_config = OfflineTtsConfig {
            model,
            max_num_sentences: 1,
            ..OfflineTtsConfig::default()
        };
        let tts = OfflineTts::create(&tts_config).ok_or_else(|| {
            TtsError::Engine(
                "sherpa-onnx could not create the synthesiser from these model files".to_owned(),
            )
        })?;
        Ok(Self {
            tts,
            speed: config.speed,
            speaker_id: config.speaker_id,
            info: config.engine_info(SHERPA_ONNX_CRATE_VERSION),
        })
    }
}

impl TtsEngine for SherpaTts {
    fn synthesize(&mut self, text: &str, cancel: &CancelFlag) -> Result<PcmChunk, TtsError> {
        // The wrapper panics on a NUL byte, and model output is not trusted.
        let text: String = text.chars().filter(|c| *c != '\0').collect();
        if text.trim().is_empty() {
            return Err(TtsError::EmptyText);
        }
        if cancel.is_cancelled() {
            return Err(TtsError::Cancelled);
        }
        let generation = GenerationConfig {
            speed: self.speed,
            sid: self.speaker_id,
            ..GenerationConfig::default()
        };
        let stop = cancel.clone();
        let audio = self.tts.generate_with_config(
            &text,
            &generation,
            Some(move |_samples: &[f32], _progress: f32| !stop.is_cancelled()),
        );
        if cancel.is_cancelled() {
            return Err(TtsError::Cancelled);
        }
        let audio = audio
            .ok_or_else(|| TtsError::Engine("the synthesiser returned no audio".to_owned()))?;
        let sample_rate = u32::try_from(audio.sample_rate())
            .ok()
            .filter(|rate| *rate > 0)
            .ok_or_else(|| {
                TtsError::Engine("the synthesiser reported no sample rate".to_owned())
            })?;
        let samples = audio.samples().to_vec();
        if samples.is_empty() {
            return Err(TtsError::Engine(
                "the synthesiser returned an empty buffer".to_owned(),
            ));
        }
        Ok(PcmChunk {
            samples,
            sample_rate,
        })
    }

    fn info(&self) -> EngineInfo {
        self.info.clone()
    }
}
