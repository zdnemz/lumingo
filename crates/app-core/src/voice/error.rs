//! Why the voice loop could not start or could not go on.

/// An error of the voice loop. A message never carries learner text, a prompt, a
/// reply or a key.
#[derive(Debug, thiserror::Error)]
pub enum VoiceError {
    #[error("the unit has no roleplay activity{}", activity_suffix(.0))]
    NoScenario(Option<String>),
    #[error("the voice loop needs {0}")]
    Missing(&'static str),
    #[error("audio: {0}")]
    Audio(#[from] audio_io::SessionError),
    #[error("audio device: {0}")]
    Device(#[from] audio_io::DeviceError),
    #[error("playback: {0}")]
    Playback(#[from] audio_io::PlaybackError),
    #[error("a speech worker could not start: {0}")]
    Worker(#[from] speech::WorkerError),
    #[error("the {engine} engine is not available: {reason}")]
    EngineUnavailable {
        engine: &'static str,
        reason: String,
    },
    #[error("the {engine} engine did not load within {seconds} s")]
    EngineLoadTimeout { engine: &'static str, seconds: u64 },
    #[error("the endpointing settings are not valid: {0}")]
    Endpoint(#[from] speech::EndpointConfigError),
    #[error("a voice loop setting is not valid: {0}")]
    Settings(&'static str),
    #[error("storage: {0}")]
    Storage(#[from] storage::StorageError),
    #[error("the voice loop has stopped")]
    Stopped,
}

fn activity_suffix(activity: &Option<String>) -> String {
    match activity {
        Some(id) => format!(" named {id}"),
        None => String::new(),
    }
}

pub type VoiceResult<T> = Result<T, VoiceError>;
