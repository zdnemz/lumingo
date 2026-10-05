use serde::{Deserialize, Serialize};

use crate::{CancelFlag, EngineInfo, SttError};

/// One recognised word with its place in the audio.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranscriptWord {
    pub text: String,
    pub start_ms: u32,
    pub end_ms: u32,
    pub confidence: Option<f32>,
}

/// What the engine heard in one utterance.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Transcript {
    pub text: String,
    /// Empty when the engine gives no word timings.
    pub words: Vec<TranscriptWord>,
    pub duration_ms: u32,
}

/// 16 kHz mono speech in, text out.
///
/// `feed` may be called many times before `finish`, so the same trait serves
/// streaming engines, which decode while audio arrives, and engines that
/// decode a whole utterance at the end.
pub trait SttEngine: Send {
    fn start(&mut self) -> Result<(), SttError>;
    fn feed(&mut self, samples: &[f32]) -> Result<(), SttError>;
    fn finish(&mut self, cancel: &CancelFlag) -> Result<Transcript, SttError>;
    fn info(&self) -> EngineInfo;
}
