//! `GET /api/diagnostics`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::common::HardwareProfile;
use super::providers::ProviderInfo;

/// Nearest-rank percentiles of a set of measurements. `None` when there are no
/// measurements: nothing is estimated.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Percentiles {
    pub count: u32,
    pub p50_ms: Option<f64>,
    pub p95_ms: Option<f64>,
}

/// Latency of one measured step over the last 30 days.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LatencyStat {
    /// For example `stt_ms` or `e2e_ms`.
    pub metric: String,
    pub stats: Percentiles,
}

/// The newest provider calls (at most 100).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LlmCallStats {
    pub calls: u32,
    /// Calls that did not end `ok` or `repaired`.
    pub failures: u32,
    pub ttft: Percentiles,
    pub total: Percentiles,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CurriculumDiagnostics {
    pub unit_count: u32,
    pub issue_count: u32,
    pub content_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DiagnosticsReport {
    pub server_version: String,
    pub dev_mode: bool,
    pub uptime_ms: u64,
    /// The address the server listens on, when the server told the core.
    pub server_address: Option<String>,
    pub data_dir: String,
    /// `None` while the program logs to its console window only.
    pub log_folder: Option<String>,
    pub curriculum_dir: String,
    /// The database schema version.
    pub schema_version: i64,
    pub hardware: HardwareProfile,
    /// The active provider with the capabilities of its last test, which
    /// include the structured-output level.
    pub provider: Option<ProviderInfo>,
    pub curriculum: CurriculumDiagnostics,
    pub latency: Vec<LatencyStat>,
    pub llm: LlmCallStats,
}
