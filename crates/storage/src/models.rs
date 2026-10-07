//! The plain types that cross the repository boundary. Other crates never see
//! `sqlx` rows or SQL; they pass these structs and typed enums.
//!
//! Every text value the schema constrains with CHECK has a matching enum here,
//! so an unknown stored value is a typed error, not a silent string.

use std::fmt;

/// A text value did not match any known variant of an enum column.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown value {value:?} for {kind}")]
pub struct UnknownValue {
    pub kind: &'static str,
    pub value: String,
}

/// Declares a string enum stored as TEXT with a CHECK in the schema.
///
/// Each variant gets `as_str` (the stored form) and `FromStr`; the `FromStr`
/// error is [`UnknownValue`], so a row written by a newer build is refused with
/// the offending value and the enum it belongs to.
macro_rules! text_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            /// The exact TEXT stored in the database.
            pub const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $text),+ }
            }

            /// Every variant, for validation and UI lists.
            pub const ALL: &'static [$name] = &[$(Self::$variant),+];
        }

        impl std::str::FromStr for $name {
            type Err = UnknownValue;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                match value {
                    $($text => Ok(Self::$variant),)+
                    other => Err(UnknownValue { kind: stringify!($name), value: other.to_owned() }),
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

text_enum! {
    /// A CEFR level, as stored in `units.level`, `assessment_attempts.level` and
    /// `skill_estimates.level`. `PreA1` exists only for estimates.
    Level {
        PreA1 => "pre-A1",
        A1 => "A1",
        A2 => "A2",
        B1 => "B1",
        B2 => "B2",
        C1 => "C1",
        C2 => "C2",
    }
}

text_enum! {
    /// What kind of session this is. Drives which tables get rows.
    SessionKind {
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

text_enum! {
    /// A session's lifecycle status.
    SessionStatus {
        Active => "active",
        Completed => "completed",
        Aborted => "aborted",
    }
}

text_enum! {
    /// Who produced a turn.
    TurnRole {
        Learner => "learner",
        Tutor => "tutor",
    }
}

text_enum! {
    /// How a turn was produced. `NoInput` is a tutor turn with no learner input,
    /// for example the opening line.
    InputMode {
        Voice => "voice",
        Text => "text",
        NoInput => "none",
    }
}

text_enum! {
    /// The conversation mode of a session, when one applies.
    SessionMode {
        Fluency => "fluency",
        Accuracy => "accuracy",
    }
}

text_enum! {
    /// How much an error gets in the way, from the `turn_analysis` contract.
    Severity {
        Minor => "minor",
        Major => "major",
        Blocking => "blocking",
    }
}

text_enum! {
    /// Which of the four skills a row trains or scores.
    Skill {
        Listening => "listening",
        Speaking => "speaking",
        Reading => "reading",
        Writing => "writing",
    }
}

text_enum! {
    /// Which scorer produced an attempt.
    Scorer {
        Deterministic => "deterministic",
        RubricLlm => "rubric_llm",
        PronEngine => "pron_engine",
    }
}

text_enum! {
    /// The lifecycle of an attempt. Productive responses wait in `PendingLlm`
    /// until a provider is reachable.
    AttemptStatus {
        Scored => "scored",
        PendingLlm => "pending_llm",
        Insufficient => "insufficient",
        NeedsReview => "needs_review",
        Rejected => "rejected",
    }
}

text_enum! {
    /// Where a response came from. Only `Authored` counts toward a level
    /// estimate; the other two never do.
    AttemptOrigin {
        Authored => "authored",
        Generated => "generated",
        FreeMode => "free_mode",
    }
}

text_enum! {
    /// The kind of evidence stored for an attempt.
    EvidenceKind {
        ResponseText => "response_text",
        Quote => "quote",
        Metric => "metric",
        ScorerReason => "scorer_reason",
    }
}

text_enum! {
    /// The UI language of the one profile.
    UiLanguage {
        Id => "id",
        En => "en",
    }
}

text_enum! {
    /// How much help the learner wants in Indonesian.
    L1HelpMode {
        Auto => "auto",
        On => "on",
        Off => "off",
    }
}

/// A profile, the single learner row of v1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub id: i64,
    pub display_name: String,
    pub ui_language: UiLanguage,
    pub l1: String,
    pub l1_help_mode: L1HelpMode,
    pub created_at: String,
}

/// What to write when creating the profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewProfile {
    pub display_name: String,
    pub ui_language: UiLanguage,
    pub l1: String,
    pub l1_help_mode: L1HelpMode,
    pub created_at: String,
}

/// A session row.
#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub id: i64,
    pub profile_id: i64,
    pub kind: SessionKind,
    pub unit_id: Option<String>,
    pub activity_id: Option<String>,
    pub mode: Option<SessionMode>,
    pub status: SessionStatus,
    pub provider_profile_id: Option<i64>,
    pub summary_json: Option<String>,
    pub app_version: String,
    pub started_at: String,
    pub ended_at: Option<String>,
}

/// What to write when starting a session.
#[derive(Debug, Clone, PartialEq)]
pub struct NewSession {
    pub profile_id: i64,
    pub kind: SessionKind,
    pub unit_id: Option<String>,
    pub activity_id: Option<String>,
    pub mode: Option<SessionMode>,
    pub provider_profile_id: Option<i64>,
    pub app_version: String,
    pub started_at: String,
}

/// A turn: a spoken turn, a chat message, or a writing draft.
#[derive(Debug, Clone, PartialEq)]
pub struct Turn {
    pub id: i64,
    pub session_id: i64,
    pub seq: i64,
    pub role: TurnRole,
    pub input_mode: InputMode,
    pub text: String,
    pub stt_text: Option<String>,
    pub edited_by_learner: bool,
    pub speech_ms: Option<i64>,
    pub pause_ms: Option<i64>,
    pub word_count: Option<i64>,
    pub created_at: String,
}

/// What to write when adding a turn.
#[derive(Debug, Clone, PartialEq)]
pub struct NewTurn {
    pub session_id: i64,
    pub seq: i64,
    pub role: TurnRole,
    pub input_mode: InputMode,
    pub text: String,
    pub stt_text: Option<String>,
    pub edited_by_learner: bool,
    pub speech_ms: Option<i64>,
    pub pause_ms: Option<i64>,
    pub word_count: Option<i64>,
    pub created_at: String,
}

/// One analysis of one turn. The JSON has already passed its contract
/// validation in `llm-client` before it reaches this crate.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnAnalysis {
    pub turn_id: i64,
    pub analysis_json: String,
    pub contract_version: String,
    pub ladder_level: i64,
    pub model: String,
    pub created_at: String,
}

/// One error event to write for an analysed turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewErrorEvent {
    pub turn_id: i64,
    pub profile_id: i64,
    pub category: String,
    pub quote: String,
    pub correction: String,
    pub severity: Severity,
    pub addressed: bool,
    pub created_at: String,
}

/// A row read back from `error_events`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorEvent {
    pub id: i64,
    pub turn_id: i64,
    pub profile_id: i64,
    pub category: String,
    pub quote: String,
    pub correction: String,
    pub severity: Severity,
    pub addressed: bool,
    pub created_at: String,
}

/// One row of the per-category tally in `error_stats`. The numbers survive a
/// session delete, so the pattern outlives the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorStat {
    pub profile_id: i64,
    pub category: String,
    pub count: i64,
    pub last_seen: Option<String>,
}

/// One scored dimension of one response.
#[derive(Debug, Clone, PartialEq)]
pub struct NewAttempt {
    pub profile_id: i64,
    pub session_id: Option<i64>,
    pub unit_id: Option<String>,
    pub activity_id: String,
    pub activity_type: String,
    /// One id per submitted response; the dimension rows of one response share it.
    pub response_id: String,
    pub origin: AttemptOrigin,
    pub level: Level,
    pub skill: Skill,
    pub dimension: String,
    pub scorer: Scorer,
    pub scorer_version: String,
    pub raw_score: Option<f64>,
    pub max_score: Option<f64>,
    pub normalized: Option<f64>,
    pub confidence: Option<f64>,
    pub status: AttemptStatus,
    pub counts_toward_estimate: bool,
    pub created_at: String,
}

/// A row read back from `assessment_attempts`.
#[derive(Debug, Clone, PartialEq)]
pub struct Attempt {
    pub id: i64,
    pub profile_id: i64,
    pub session_id: Option<i64>,
    pub unit_id: Option<String>,
    pub activity_id: String,
    pub activity_type: String,
    pub response_id: String,
    pub origin: AttemptOrigin,
    pub level: Level,
    pub skill: Skill,
    pub dimension: String,
    pub scorer: Scorer,
    pub scorer_version: String,
    pub raw_score: Option<f64>,
    pub max_score: Option<f64>,
    pub normalized: Option<f64>,
    pub confidence: Option<f64>,
    pub status: AttemptStatus,
    pub counts_toward_estimate: bool,
    pub created_at: String,
}

/// One piece of evidence for an attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewEvidence {
    pub attempt_id: i64,
    pub kind: EvidenceKind,
    /// Learner text, a quote, or a scorer reason. `None` for a metric row.
    pub content: Option<String>,
    /// Structured metric detail. Must be valid JSON when present.
    pub data_json: Option<String>,
    pub created_at: String,
}

/// A row read back from `assessment_evidence`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evidence {
    pub id: i64,
    pub attempt_id: i64,
    pub kind: EvidenceKind,
    pub content: Option<String>,
    pub data_json: Option<String>,
    pub created_at: String,
}

/// A productive response waiting for a provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPendingScoring {
    pub attempt_id: i64,
    pub payload_json: String,
    pub created_at: String,
}

/// One row of the pending queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingScoring {
    pub id: i64,
    pub attempt_id: i64,
    pub payload_json: String,
    pub tries: i64,
    pub created_at: String,
}

/// A row read back from `audio_clips`. The file on disk is the caller's to
/// delete before the session row is deleted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioClip {
    pub id: i64,
    pub turn_id: i64,
    pub path: String,
    pub duration_ms: i64,
    pub created_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn enums_round_trip_through_their_stored_text() {
        for level in Level::ALL {
            assert_eq!(Level::from_str(level.as_str()).unwrap(), *level);
        }
        for kind in SessionKind::ALL {
            assert_eq!(SessionKind::from_str(kind.as_str()).unwrap(), *kind);
        }
        assert_eq!(Level::from_str("pre-A1").unwrap(), Level::PreA1);
        assert_eq!(InputMode::from_str("none").unwrap(), InputMode::NoInput);
    }

    #[test]
    fn an_unknown_stored_value_names_the_kind_and_the_value() {
        let error = Level::from_str("C3").unwrap_err();
        assert_eq!(error.kind, "Level");
        assert_eq!(error.value, "C3");
        assert!(error.to_string().contains("C3"));
    }
}
