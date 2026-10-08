//! Data operations: delete and export.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

/// `DELETE /api/sessions/{id}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DeleteSessionResult {
    /// Recordings that were deleted from disk with the session.
    pub audio_files_removed: u32,
    /// Whether the database file was rewritten afterwards so the deleted text
    /// does not stay in unused pages. `false` means that step failed and the
    /// text is gone from every table but may still sit in the file.
    pub compacted: bool,
}

/// `DELETE /api/data`: everything about the learner is removed. Provider
/// profiles, settings and the unit index are not learning data and stay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DeleteDataResult {
    pub audio_files_removed: u32,
    pub compacted: bool,
}

/// `GET /api/export`: the learner's personal data as JSON. It never contains a
/// key, not even the last four characters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExportBundle {
    /// Grows when the layout of `data` changes.
    pub format_version: u32,
    /// RFC 3339.
    pub exported_at: String,
    pub app_version: String,
    /// The sections: profile, settings, providers, sessions, attempts, progress,
    /// estimates, game. It is the stored rows as they are, so the layout follows
    /// the database and is described by `docs/DATA_MODEL.md`.
    #[ts(type = "Record<string, unknown>")]
    pub data: Value,
}
