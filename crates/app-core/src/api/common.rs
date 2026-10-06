//! Types shared by several route groups.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::mirror::api_enum;

/// The body of every error answer. `error` is the code the UI switches on and
/// `message` is a sentence in English for logs and for the "details" line. The
/// field is called `error` because the request guard of `apps/server` already
/// answers `{"error":"host_not_allowed"}` and the UI reads the same field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ApiErrorBody {
    pub error: ErrorCode,
    pub message: String,
    /// For `not_available`: the part of the program that is missing, so the UI
    /// can show the matching note. The message names what exactly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub feature: Option<Feature>,
}

/// Why a request failed, as a closed list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ErrorCode {
    NotFound,
    InvalidInput,
    Conflict,
    ReadOnly,
    Busy,
    NotAvailable,
    /// A model download was asked for without the licence being accepted, or with
    /// a licence other than the one that was shown.
    LicenceNotAccepted,
    ProviderNotConfigured,
    ShuttingDown,
    Storage,
    Internal,
}

/// A part of the program that this executable does not contain (yet). The
/// snapshot lists them, so the UI can say so instead of showing a button that
/// fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Feature {
    /// Starting, pausing and stopping lessons, conversations and drills.
    Sessions,
    /// Submitting an answer to an activity.
    Activities,
    /// Writing workshop, graded reading and text chat.
    FreeModes,
    /// Microphone, speakers and speech models.
    Speech,
    /// Downloading and listing speech models.
    Models,
}

impl Feature {
    /// Every feature, in the order the snapshot lists them.
    pub const ALL: [Feature; 5] = [
        Feature::Sessions,
        Feature::Activities,
        Feature::FreeModes,
        Feature::Speech,
        Feature::Models,
    ];

    /// A short English name for messages.
    pub fn label(self) -> &'static str {
        match self {
            Self::Sessions => "sessions",
            Self::Activities => "activity answers",
            Self::FreeModes => "the free modes",
            Self::Speech => "speech",
            Self::Models => "model downloads",
        }
    }
}

api_enum! {
    /// Interface language.
    UiLanguage <=> storage::UiLanguage { Id => "id", En => "en" }
}

api_enum! {
    /// Whether the tutor may use the learner's first language to help.
    L1HelpMode <=> storage::L1HelpMode { Auto => "auto", On => "on", Off => "off" }
}

/// What this machine has, as measured. A value is `None` when the operating
/// system did not give it, and nothing is guessed in its place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HardwareProfile {
    /// Installed physical memory in bytes.
    pub ram_total_bytes: Option<u64>,
    /// Logical processors this process may use.
    pub logical_cores: Option<u32>,
    /// The floor from the product requirements: memory in bytes.
    pub minimum_ram_bytes: u64,
    /// The floor from the product requirements: logical processors.
    pub minimum_logical_cores: u32,
    /// Whether both measured values reach the floor. `None` when either value is
    /// unknown.
    pub meets_minimum: Option<bool>,
}
