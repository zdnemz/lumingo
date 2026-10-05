//! Engine traits and the small types every speech crate shares.
//!
//! The traits are deliberately narrow. A concrete engine (sherpa-onnx, whisper,
//! a test double) sits behind one of them, so the rest of the program never
//! names a vendor. Adapters that need a native library live in other crates
//! behind cargo features that are off by default.
#![forbid(unsafe_code)]

mod cancel;
mod endpoint;
mod error;
mod info;
mod stt;
mod stt_worker;
mod tts;
mod unavailable;
mod vad;

pub use cancel::CancelFlag;
pub use endpoint::{
    EndReason, EndpointConfig, EndpointConfigError, EndpointEvent, Endpointer, MAX_END_SILENCE_MS,
    MIN_END_SILENCE_MS, SegmentEvent, Utterance, UtteranceSegmenter,
};
pub use error::{SttError, TtsError, VadError, WorkerError};
pub use info::EngineInfo;
pub use stt::{SttEngine, Transcript, TranscriptWord};
pub use stt_worker::{
    SttEvent, SttJob, SttWorker, SttWorkerConfig, SttWorkerStats, SubmitError as SttSubmitError,
};
pub use tts::{PcmChunk, TtsEngine};
pub use unavailable::UnavailableEngine;
pub use vad::Vad;

/// The sample rate every recognition and voice-activity engine is fed at.
pub const SPEECH_SAMPLE_RATE: u32 = 16_000;
