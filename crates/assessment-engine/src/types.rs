use serde::{Deserialize, Serialize};

/// A CEFR scale name. It names a band of ability. Only the estimator may attach
/// one to a learner, and always as an estimate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Level {
    A1,
    A2,
    B1,
    B2,
    C1,
    C2,
}

impl Level {
    pub const ALL: [Self; 6] = [Self::A1, Self::A2, Self::B1, Self::B2, Self::C1, Self::C2];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::A1 => "A1",
            Self::A2 => "A2",
            Self::B1 => "B1",
            Self::B2 => "B2",
            Self::C1 => "C1",
            Self::C2 => "C2",
        }
    }
}

/// The four skills that carry a level estimate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Skill {
    Listening,
    Speaking,
    Reading,
    Writing,
}

impl Skill {
    pub const ALL: [Self; 4] = [
        Self::Listening,
        Self::Speaking,
        Self::Reading,
        Self::Writing,
    ];

    /// Speaking and writing cannot be secured by objective items alone.
    pub fn is_productive(self) -> bool {
        matches!(self, Self::Speaking | Self::Writing)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scorer {
    Deterministic,
    RubricLlm,
    PronEngine,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptStatus {
    Scored,
    /// Waiting for a provider to score it.
    PendingLlm,
    /// The cross-checks disagreed.
    NeedsReview,
    Insufficient,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// An item written for a unit, a checkpoint or the placement test.
    Authored,
    /// Extra practice generated inside a unit.
    Generated,
    /// Text chat, writing workshop, graded reading, free conversation.
    FreeMode,
}

/// One scored dimension of one response, as stored. Rows that belong to the same
/// response share a `response_id`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attempt {
    /// Rubric-scored responses share this across their dimensions. A deterministic
    /// item has none and is its own observation.
    pub response_id: Option<String>,
    pub skill: Skill,
    /// The level of the unit or placement block the item belongs to.
    pub level: Level,
    pub activity_id: String,
    pub session_id: String,
    pub scorer: Scorer,
    /// 0 to 1.
    pub normalized: f64,
    /// 0 to 1.
    pub confidence: f64,
    pub status: AttemptStatus,
    pub origin: Origin,
    pub counts_toward_estimate: bool,
    /// A productive task scored with a rubric: `guided_speaking`, a scored
    /// `roleplay`, `guided_writing` or `mediation`.
    pub productive_response: bool,
    /// Unix seconds.
    pub created_at: i64,
}
