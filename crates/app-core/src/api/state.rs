//! `GET /api/state` and the first WebSocket message.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::common::{Feature, HardwareProfile};
use super::providers::ProviderInfo;
use super::settings::Settings;

/// Full picture of the program at one moment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StateSnapshot {
    pub server_version: String,
    pub dev_mode: bool,
    pub uptime_ms: u64,
    pub settings: Settings,
    /// The active provider, or `None` when none is configured.
    pub provider: Option<ProviderInfo>,
    pub hardware: HardwareProfile,
    /// Parts of the program this executable does not contain. Empty once all are
    /// built in.
    pub unavailable: Vec<Feature>,
}
