//! Speech route group: speaking a stored text, listing audio devices, testing them.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// `POST /api/tts/speak`. The text is named by reference to something the server
/// stored; a request never carries text to speak.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "source", rename_all = "snake_case")]
#[ts(export)]
pub enum SpeakRequest {
    /// A tutor turn of a stored session.
    Turn { session_id: i64, turn_seq: i64 },
    /// A generated reading text of a stored reading session.
    Reading { session_id: i64, content_id: i64 },
    /// The audio lines of an activity of the unit session that is running. Each
    /// request counts as one play of the item, so a listening set that allows no
    /// more plays refuses it.
    Activity {
        session_id: i64,
        activity_id: String,
    },
}

/// The answer to a speak request. The sentences are reported as events while
/// they are spoken; `POST /api/tutor/stop-speaking` stops them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SpeakAccepted {
    /// How many sentences will be spoken.
    pub sentences: u32,
}

/// An audio device of this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AudioDeviceView {
    /// The backend's own identifier. Only the backend can read it.
    pub id: String,
    pub name: String,
    pub is_default: bool,
    /// The learner picked this device earlier.
    pub is_selected: bool,
    pub sample_rate: u32,
    pub channels: u32,
}

/// `GET /api/audio/devices`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AudioDevices {
    pub inputs: Vec<AudioDeviceView>,
    pub outputs: Vec<AudioDeviceView>,
}

/// `POST /api/audio/test`: the microphone check of the first-run wizard. The
/// server records for `duration_ms`, reports the level through `MicLevel`
/// events, and may play the recording back. The recording stays in memory and is
/// dropped when the test ends; it is never stored and never sent anywhere.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AudioTestRequest {
    /// How long to record, 500 to 5000 ms.
    pub duration_ms: u32,
    /// Play the recording back through the speakers afterwards.
    #[serde(default)]
    pub play_back: bool,
    /// The microphone to test. Without one, the remembered or the default one.
    #[serde(default)]
    pub input_id: Option<String>,
    /// The speaker to play back on. Without one, the remembered or the default one.
    #[serde(default)]
    pub output_id: Option<String>,
    /// Remember the devices that were used when the test worked.
    #[serde(default)]
    pub remember: bool,
}

/// What the test heard.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AudioTestReport {
    pub input_device: String,
    pub output_device: Option<String>,
    /// How much audio was recorded, in ms.
    pub recorded_ms: u32,
    /// The loudest sample, 0 to 1.
    pub peak: f32,
    /// The root mean square of the recording, 0 to 1.
    pub rms: f32,
    /// Almost nothing was heard: the microphone may be muted or the wrong one.
    pub silent: bool,
    /// Samples at full scale: the input is too loud.
    pub clipped: bool,
    pub played_back: bool,
}
