//! `GET /api/state` and the first WebSocket message.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::common::{Feature, HardwareProfile};
use super::engines::EngineView;
use super::providers::ProviderInfo;
use super::sessions::ActiveSessionView;
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
    /// The session that is running, or `None`. At most one runs at a time. A page
    /// that was reloaded rebuilds the conversation from it.
    pub active_session: Option<ActiveSessionView>,
    /// The speech and audio engines and where each stands.
    pub engines: Vec<EngineView>,
    /// Parts of the program this executable does not provide, computed from what
    /// actually loaded. Empty when all are available.
    pub unavailable: Vec<Feature>,
}
