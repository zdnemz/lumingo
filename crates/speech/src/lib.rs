//! Engine traits and the small types every speech crate shares.
//!
//! The traits are deliberately narrow. A concrete engine (sherpa-onnx, whisper,
//! a test double) sits behind one of them, so the rest of the program never
//! names a vendor. Adapters that need a native library live in other crates
//! behind cargo features that are off by default.
#![forbid(unsafe_code)]

mod cancel;
mod chunker;
mod endpoint;
mod error;
mod info;
mod stt;
mod tts;
mod unavailable;
mod vad;

pub use cancel::CancelFlag;
pub use chunker::SentenceChunker;
pub use endpoint::{EndpointConfig, EndpointEvent, Endpointer};
pub use error::{SttError, TtsError, VadError};
pub use info::EngineInfo;
pub use stt::{SttEngine, Transcript, TranscriptWord};
pub use tts::{PcmChunk, TtsEngine};
pub use unavailable::UnavailableEngine;
pub use vad::Vad;

/// The sample rate every recognition and voice-activity engine is fed at.
pub const SPEECH_SAMPLE_RATE: u32 = 16_000;
