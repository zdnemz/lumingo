//! The speech and audio engines as the snapshot and the `EngineStatus` event show them.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Which engine a status is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum EngineId {
    /// The microphone.
    AudioInput,
    /// The speakers.
    AudioOutput,
    /// Voice activity detection.
    Vad,
    /// Speech recognition.
    Stt,
    /// Speech synthesis.
    Tts,
    /// The phoneme model for pronunciation scoring.
    Pron,
}

/// Where an engine stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum EngineState {
    /// Not in this executable, not configured, or its files are missing. The
    /// detail says which.
    Unavailable,
    /// Built in and configured. The model loads when a voice session starts.
    Configured,
    /// A model is loading now.
    Loading,
    /// A model loaded and ran in this run.
    Ready,
    /// The last attempt to load or use it failed. The detail says why.
    Failed,
}

/// Which engine and model produced a result, as the engine reports itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EngineModelInfo {
    pub id: String,
    pub version: String,
    /// SHA-256 over the model files, when the engine loads a model.
    pub model_checksum: Option<String>,
}

/// One engine and its state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EngineView {
    pub id: EngineId,
    pub state: EngineState,
    /// One sentence for the learner: why it is unavailable or failed, or what is
    /// configured. It holds no path and no learner text.
    pub detail: String,
    /// Set once the engine has loaded in this run.
    pub model: Option<EngineModelInfo>,
}
