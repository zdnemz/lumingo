use thiserror::Error;

/// Why a worker thread could not be started.
#[derive(Debug, Error)]
pub enum WorkerError {
    #[error("the {name} worker thread could not be started: {source}")]
    Spawn {
        name: &'static str,
        source: std::io::Error,
    },
    #[error("a queue capacity of zero is not usable")]
    ZeroCapacity,
}

#[derive(Debug, Error)]
pub enum SttError {
    #[error("the speech recognition engine is not available: {reason}")]
    Unavailable { reason: String },
    #[error("speech recognition was cancelled")]
    Cancelled,
    #[error("speech recognition failed: {0}")]
    Engine(String),
}

#[derive(Debug, Error)]
pub enum TtsError {
    #[error("the speech synthesis engine is not available: {reason}")]
    Unavailable { reason: String },
    #[error("speech synthesis was cancelled")]
    Cancelled,
    #[error("the text to speak is empty")]
    EmptyText,
    #[error("speech synthesis failed: {0}")]
    Engine(String),
}

#[derive(Debug, Error)]
pub enum VadError {
    #[error("the voice activity engine is not available: {reason}")]
    Unavailable { reason: String },
    #[error("a frame must hold {expected} samples, got {got}")]
    FrameSize { expected: usize, got: usize },
    #[error("voice activity detection failed: {0}")]
    Engine(String),
}
