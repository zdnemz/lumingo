//! Every request, response and event type of the HTTP and WebSocket API.
//!
//! They are defined here once. `cargo test -p app-core` writes the TypeScript
//! declarations to `apps/web/src/generated/` (see `.cargo/config.toml`), and CI
//! fails when the checked-in files differ.

mod activities;
mod common;
mod data;
mod diagnostics;
mod engines;
mod events;
mod feedback;
mod free;
mod game;
mod inspector;
pub(crate) mod mirror;
mod models;
mod progress;
mod providers;
mod sessions;
mod settings;
mod speech;
mod state;
mod units;

pub use activities::{
    ActivityAnswer, ActivityBody, ActivityPresentation, ActivityUnavailable, AudioLineView,
    NextActivity, PairChoice, PairPrompt, QuestionPrompt, SubmitActivityRequest,
    SubmitActivityResponse,
};
pub use common::{ApiErrorBody, ErrorCode, Feature, HardwareProfile, L1HelpMode, UiLanguage};
pub use data::{DeleteDataResult, DeleteSessionResult, ExportBundle};
pub use diagnostics::{
    CurriculumDiagnostics, DiagnosticsReport, LatencyStat, LlmCallStats, Percentiles,
};
pub use engines::{EngineId, EngineModelInfo, EngineState, EngineView};
pub use events::ServerEvent;
pub use feedback::{
    ActivityOutcomeView, ActivityResultView, CheckpointRowView, ContentPointView,
    ConversationSummaryView, DeterministicFeedbackView, DraftComparisonView, DraftErrorView,
    DraftFeedbackView, DraftResolution, DraftStatusView, EarlierErrorView, ErrorFindingView,
    ErrorPatternView, ErrorSeverity, EvidenceStatusView, FeedbackView, ItemFeedbackView,
    ItemOutcomeView, ObjectiveEvidenceView, PronFindingsView, PronWordView, QuestionResultView,
    ReadingScoreView, RubricDimensionView, RubricFeedbackView, TurnAnalysisView, UnitSummaryView,
};
pub use free::{
    AnswerReadingRequest, AuthoredSetView, DraftAccepted, GenerateReadingRequest, GlossEntryView,
    ReadingAnswered, ReadingFallbackReasonView, ReadingOutcomeView, ReadingQuestionView,
    ReadingTarget, ReadingTextView, SubmitDraftRequest,
};
pub use game::{
    CosmeticKind, CosmeticSlot, CosmeticView, EquipRequest, EquippedView, GameState,
    PracticeOutcome, RankView, StreakDayView, StreakView, XpBySourceView, XpSourceKind,
};
pub use inspector::{InspectorReport, PayloadEntryView, PayloadOutcomeView};
pub use models::{
    DownloadRequest, DownloadStarted, DownloadState, Downloadable, InstalledModelView, LicenceView,
    ModelList, ModelRoleView, ModelView,
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
pub use sessions::{
    Ack, ActiveSessionView, EditEffect, EditResult, EditTurnRequest, EngineFaultView, FeedbackMode,
    LineRole, PushToTalkRequest, RECENT_LINE_CHARS, RECENT_LINES, SendTextRequest, SessionChannel,
    SessionEnded, SessionLife, StartSessionRequest, StopRequest, TopicChoice, TurnAccepted,
    TurnLine, TurnPhase,
};
pub use settings::{AdaptiveTiming, Settings};
pub use speech::{
    AudioDeviceView, AudioDevices, AudioTestReport, AudioTestRequest, SpeakAccepted, SpeakRequest,
};
pub use state::StateSnapshot;
pub use units::{UnitDetail, UnitIssue, UnitList, UnitSummary};
