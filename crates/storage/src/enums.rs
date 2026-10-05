//! The closed vocabularies that the schema enforces with CHECK constraints.
//!
//! Each enum is the Rust side of one CHECK list in `0001_init.sql`. Writing
//! through the enum means a typo is a compile error, and reading a value this
//! build does not know is reported as `InvalidStored` instead of being guessed.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::{Result, StorageError};

/// Implemented by every enum below. Lets row decoding stay generic.
pub(crate) trait DbEnum: Sized {
    fn parse(text: &str) -> Option<Self>;
}

pub(crate) fn parse_enum<E: DbEnum>(column: &'static str, text: &str) -> Result<E> {
    E::parse(text).ok_or(StorageError::InvalidStored { column })
}

macro_rules! string_enum {
    (
        $(#[$meta:meta])*
        $name:ident { $( $(#[$vmeta:meta])* $variant:ident => $text:literal ),+ $(,)? }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum $name {
            $( $(#[$vmeta])* #[serde(rename = $text)] $variant ),+
        }

        impl $name {
            /// Every value, in declaration order.
            pub const ALL: &'static [$name] = &[ $( $name::$variant ),+ ];

            /// The text stored in the database.
            pub fn as_str(&self) -> &'static str {
                match self { $( $name::$variant => $text ),+ }
            }
        }

        impl DbEnum for $name {
            fn parse(text: &str) -> Option<Self> {
                match text { $( $text => Some($name::$variant), )+ _ => None }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

string_enum! {
    /// Interface language of a profile.
    UiLanguage { Id => "id", En => "en" }
}

string_enum! {
    /// Whether the tutor may use the learner's first language for help.
    L1HelpMode { Auto => "auto", On => "on", Off => "off" }
}

string_enum! {
    /// CEFR level of a unit or an activity. Written only from authored content.
    Level { A1 => "A1", A2 => "A2", B1 => "B1", B2 => "B2", C1 => "C1", C2 => "C2" }
}

string_enum! {
    /// Level column of a stored estimate. Written only by the estimation code,
    /// never from model output.
    EstimateLevel {
        PreA1 => "pre-A1", A1 => "A1", A2 => "A2", B1 => "B1", B2 => "B2", C1 => "C1", C2 => "C2"
    }
}

string_enum! {
    /// Wire protocol of a provider profile.
    ProviderProtocol { OpenaiChat => "openai_chat", AnthropicMessages => "anthropic_messages" }
}

string_enum! {
    /// Where a provider's key is read from. The key itself is never stored.
    KeySource { Env => "env", File => "file" }
}

string_enum! {
    /// Role of an installed speech model.
    ModelRole { Vad => "vad", Stt => "stt", Tts => "tts", Pron => "pron" }
}

string_enum! {
    /// What a session is for.
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

string_enum! {
    /// Fluency or accuracy emphasis of a spoken session.
    SessionMode { Fluency => "fluency", Accuracy => "accuracy" }
}

string_enum! {
    /// Lifecycle of a session.
    SessionStatus { Active => "active", Completed => "completed", Aborted => "aborted" }
}

string_enum! {
    /// Who produced a turn.
    TurnRole { Learner => "learner", Tutor => "tutor" }
}

string_enum! {
    /// How a turn was entered.
    InputMode { Voice => "voice", Text => "text", None => "none" }
}

string_enum! {
    /// Severity of an error found in a learner turn.
    Severity { Minor => "minor", Major => "major", Blocking => "blocking" }
}

string_enum! {
    /// What the LLM generated for a session.
    GeneratedKind { ReadingPassage => "reading_passage", PracticeItems => "practice_items" }
}

string_enum! {
    /// Where an assessed item came from. Only `Authored` may count toward an estimate.
    AttemptOrigin { Authored => "authored", Generated => "generated", FreeMode => "free_mode" }
}

string_enum! {
    /// What produced a score.
    Scorer { Deterministic => "deterministic", RubricLlm => "rubric_llm", PronEngine => "pron_engine" }
}

string_enum! {
    /// State of one scored dimension.
    AttemptStatus {
        Scored => "scored",
        PendingLlm => "pending_llm",
        Insufficient => "insufficient",
        NeedsReview => "needs_review",
        Rejected => "rejected",
    }
}

string_enum! {
    /// Kind of evidence kept for an attempt.
    EvidenceKind {
        ResponseText => "response_text",
        Quote => "quote",
        Metric => "metric",
        ScorerReason => "scorer_reason",
    }
}

string_enum! {
    /// Status of a stored skill estimate.
    EstimateStatus {
        Estimated => "estimated",
        InsufficientEvidence => "insufficient_evidence",
        PlacementOnly => "placement_only",
    }
}

string_enum! {
    /// Where an XP award came from. The session kinds, so free-mode activity can
    /// earn XP, plus `Bonus` for awards that belong to no session. XP is
    /// cosmetic and never feeds an estimate.
    XpSourceKind {
        Lesson => "lesson",
        Conversation => "conversation",
        TextChat => "text_chat",
        Writing => "writing",
        Reading => "reading",
        Drill => "drill",
        Review => "review",
        Checkpoint => "checkpoint",
        Placement => "placement",
        Bonus => "bonus",
    }
}

string_enum! {
    /// Kind of call recorded in the technical log.
    LlmCallType {
        TutorTurn => "tutor_turn",
        TurnAnalysis => "turn_analysis",
        RubricScore => "rubric_score",
        PracticeGen => "practice_gen",
        ReadingGen => "reading_gen",
        Explain => "explain",
        Probe => "probe",
    }
}

string_enum! {
    /// How an LLM call ended.
    LlmOutcome {
        Ok => "ok",
        Repaired => "repaired",
        InvalidOutput => "invalid_output",
        Timeout => "timeout",
        RateLimited => "rate_limited",
        Refused => "refused",
        Cancelled => "cancelled",
        Error => "error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_value_round_trips_through_its_text() {
        for level in Level::ALL {
            assert_eq!(Level::parse(level.as_str()), Some(*level));
        }
        for level in EstimateLevel::ALL {
            assert_eq!(EstimateLevel::parse(level.as_str()), Some(*level));
        }
        assert_eq!(SessionKind::parse("text_chat"), Some(SessionKind::TextChat));
        assert_eq!(SessionKind::parse("TEXT_CHAT"), None);
    }

    #[test]
    fn serde_uses_the_same_text_as_the_database() {
        let json = serde_json::to_string(&EstimateLevel::PreA1).expect("serialise");
        assert_eq!(json, "\"pre-A1\"");
        assert!(serde_json::from_str::<SessionKind>("\"free_mode_x\"").is_err());
        let known: SessionKind = serde_json::from_str("\"text_chat\"").expect("known value");
        assert_eq!(known, SessionKind::TextChat);
    }

    #[test]
    fn unknown_stored_text_is_an_error_not_a_guess() {
        let result: Result<Level> = parse_enum("units.level", "D1");
        assert!(matches!(
            result,
            Err(StorageError::InvalidStored {
                column: "units.level"
            })
        ));
    }
}
