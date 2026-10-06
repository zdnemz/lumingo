//! Models route group: what the manifest offers, what is installed, and downloads.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// What a model is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ModelRoleView {
    Vad,
    Stt,
    Tts,
    Pron,
}

/// The licence the learner reads before a download.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LicenceView {
    pub license: String,
    pub license_url: String,
    /// The licence text or use restrictions, when the manifest carries them.
    pub text: Option<String>,
    /// The licence has use restrictions that must be read in full.
    pub needs_review: bool,
}

/// Whether a model can be downloaded now, and if not, why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
#[ts(export)]
pub enum Downloadable {
    Yes,
    /// The manifest entry cannot be fetched or verified (no files, an empty
    /// checksum, no location or licence). Nothing is fetched for it.
    No {
        reason: String,
    },
}

/// One installed model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct InstalledModelView {
    pub version: String,
    pub files: u32,
    pub size_bytes: u64,
    /// One SHA-256 over every file's path and checksum.
    pub combined_sha256: String,
}

/// A model of the manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ModelView {
    pub id: String,
    pub role: ModelRoleView,
    pub engine: String,
    pub version: String,
    /// Declared size in bytes, 0 until it was measured from a real download.
    pub size_bytes: u64,
    pub licence: LicenceView,
    pub downloadable: Downloadable,
    pub installed: Option<InstalledModelView>,
    /// A download of this model is running now.
    pub downloading: bool,
    /// The owner's note about what is unverified for this entry.
    pub notes: Option<String>,
}

/// `GET /api/models`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ModelList {
    pub models: Vec<ModelView>,
}

/// `POST /api/models/{id}/download`. The licence is the one the learner was shown
/// in `GET /api/models`; a download starts only when it is the current one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DownloadRequest {
    /// The learner accepted the licence.
    pub accept_licence: bool,
    /// The licence name as shown.
    pub license: String,
    /// The licence address as shown.
    pub license_url: String,
}

/// The download started. Progress arrives as `DownloadProgress` events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DownloadStarted {
    pub model_id: String,
    pub files: u32,
}

/// Where a download stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DownloadState {
    Running,
    /// Every file arrived, matched its checksum and the model is recorded as installed.
    Done,
    Failed,
    Cancelled,
}
