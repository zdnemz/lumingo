//! Progress read models. Everything here is read from storage as stored; the
//! core computes no score, mastery or level.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use super::mirror::api_enum;

api_enum! {
    /// Where a unit stands for the learner.
    UnitStatus <=> storage::UnitStatus {
        Locked => "locked",
        Available => "available",
        InProgress => "in_progress",
        Passed => "passed",
        Skipped => "skipped",
    }
}

api_enum! {
    /// The level of a stored estimate. It is written by the estimation code from
    /// authored attempts only, and is always shown as an estimate with its status
    /// and confidence.
    EstimateLevel <=> storage::EstimateLevel {
        PreA1 => "pre-A1",
        A1 => "A1",
        A2 => "A2",
        B1 => "B1",
        B2 => "B2",
        C1 => "C1",
        C2 => "C2",
    }
}

api_enum! {
    /// Whether an estimate has enough evidence behind it.
    EstimateStatus <=> storage::EstimateStatus {
        Estimated => "estimated",
        InsufficientEvidence => "insufficient_evidence",
        PlacementOnly => "placement_only",
    }
}

api_enum! {
    /// What a session was for.
    SessionKind <=> storage::SessionKind {
        Lesson => "lesson",
        Conversation => "conversation",
        TextChat => "text_chat",
        Writing => "writing",
        Reading => "reading",
        Drill => "drill",
        Review => "review",
        Checkpoint => "checkpoint",
        Placement => "placement",
    }
}

api_enum! {
    /// How a session stands.
    SessionStatus <=> storage::SessionStatus {
        Active => "active",
        Completed => "completed",
        Aborted => "aborted",
    }
}

api_enum! {
    /// What a piece of evidence is.
    EvidenceKind <=> storage::EvidenceKind {
        ResponseText => "response_text",
        Quote => "quote",
        Metric => "metric",
        ScorerReason => "scorer_reason",
    }
}

api_enum! {
    /// What a review item is about.
    ReviewItemKind <=> storage::ReviewKind {
        Vocab => "vocab",
        Grammar => "grammar",
        Pron => "pron",
    }
}

/// A unit the learner has a progress row for. Units without a row are not here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UnitProgressView {
    pub unit_id: String,
    pub status: UnitStatus,
    /// Best checkpoint score, 0 to 1.
    pub best_checkpoint: Option<f64>,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ObjectiveMasteryView {
    /// `<unit id>/<objective id>`.
    pub objective_id: String,
    /// 0 to 1.
    pub mastery: f64,
    pub attempts: i64,
    pub last_attempt_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ErrorStatView {
    pub category: String,
    pub count: i64,
    pub last_seen: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ReviewDueView {
    pub item_kind: ReviewItemKind,
    /// `<unit id>/<item id>`.
    pub item_ref: String,
    pub due_at: String,
    pub interval_days: f64,
    pub reps: i64,
    pub lapses: i64,
}

/// The newest estimate of one skill.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SkillEstimateView {
    pub skill: String,
    /// `None` while the status says there is not enough evidence.
    pub level: Option<EstimateLevel>,
    pub status: EstimateStatus,
    pub confidence: Option<f64>,
    pub evidence_count: i64,
    /// The version of the estimation rule, so the number can be traced.
    pub algorithm_version: String,
    pub computed_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SessionSummary {
    pub id: i64,
    pub kind: SessionKind,
    pub unit_id: Option<String>,
    pub status: SessionStatus,
    pub started_at: String,
    pub ended_at: Option<String>,
}

/// `GET /api/progress`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ProgressOverview {
    pub units: Vec<UnitProgressView>,
    pub objectives: Vec<ObjectiveMasteryView>,
    pub errors: Vec<ErrorStatView>,
    /// Review items due now, most overdue first, at most 50.
    pub reviews_due: Vec<ReviewDueView>,
    /// True when more than 50 are due.
    pub reviews_due_truncated: bool,
    pub estimates: Vec<SkillEstimateView>,
    /// The latest sessions, newest first, at most 50.
    pub recent_sessions: Vec<SessionSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EvidenceView {
    pub id: i64,
    pub kind: EvidenceKind,
    /// Learner text or a quote. It is removed when the session is deleted.
    pub content: Option<String>,
    pub data: Option<Value>,
    pub created_at: String,
}

/// `GET /api/attempts/{id}/evidence`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AttemptEvidence {
    pub attempt_id: i64,
    pub activity_id: String,
    pub skill: String,
    pub dimension: String,
    pub evidence: Vec<EvidenceView>,
}
