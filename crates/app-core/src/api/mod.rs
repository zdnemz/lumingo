//! Every request, response and event type of the HTTP and WebSocket API.
//!
//! They are defined here once. `cargo test -p app-core` writes the TypeScript
//! declarations to `apps/web/src/generated/` (see `.cargo/config.toml`), and CI
//! fails when the checked-in files differ.

mod common;
mod data;
mod diagnostics;
mod events;
mod game;
pub(crate) mod mirror;
mod progress;
mod providers;
mod settings;
mod state;
mod units;

pub use common::{ApiErrorBody, ErrorCode, Feature, HardwareProfile, L1HelpMode, UiLanguage};
pub use data::{DeleteDataResult, DeleteSessionResult, ExportBundle};
pub use diagnostics::{
    CurriculumDiagnostics, DiagnosticsReport, LatencyStat, LlmCallStats, Percentiles,
};
pub use events::ServerEvent;
pub use game::{
    CosmeticKind, CosmeticSlot, CosmeticView, EquipRequest, EquippedView, GameState,
    PracticeOutcome, RankView, StreakDayView, StreakView, XpBySourceView, XpSourceKind,
};
pub use progress::{
    AttemptEvidence, ErrorStatView, EstimateLevel, EstimateStatus, EvidenceKind, EvidenceView,
    ObjectiveMasteryView, ProgressOverview, ReviewDueView, ReviewItemKind, SessionKind,
    SessionStatus, SessionSummary, SkillEstimateView, UnitProgressView, UnitStatus,
};
pub use providers::{
    ProbeFailure, ProbeFailureKind, ProbeReport, ProviderCapabilities, ProviderInfo, ProviderList,
    ProviderProtocol, ProviderSource, SaveProviderRequest, SecretText,
};
pub use settings::{AdaptiveTiming, Settings};
pub use state::StateSnapshot;
pub use units::{UnitDetail, UnitIssue, UnitList, UnitSummary};
