//! `GET /api/inspector`: the last provider requests and responses.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// How a captured request ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PayloadOutcomeView {
    Pending,
    Ok,
    HttpError,
    Failed,
    Abandoned,
}

/// One request that was sent to the provider and what came back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PayloadEntryView {
    pub id: u64,
    pub started_unix_ms: u64,
    /// Host and path. No query string, no credentials.
    pub endpoint: String,
    pub streaming: bool,
    pub request_body: String,
    pub request_truncated: bool,
    pub status: Option<u16>,
    pub response_body: Option<String>,
    pub response_truncated: bool,
    pub outcome: PayloadOutcomeView,
    pub elapsed_ms: Option<u64>,
}

/// The payload inspector. It shows what left the machine, so the learner can check
/// it. The key is removed from every entry when it is captured and headers are not
/// kept. It is not part of the export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct InspectorReport {
    /// Entries kept at most.
    pub capacity: u32,
    /// The longest request or response body kept, in bytes. A longer one is cut.
    pub max_body_bytes: u32,
    /// Entries dropped since the program started because the ring was full.
    pub dropped: u64,
    /// Oldest first.
    pub entries: Vec<PayloadEntryView>,
}
