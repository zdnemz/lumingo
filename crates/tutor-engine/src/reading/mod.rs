//! Graded reading (contract T6, S4-13).
//!
//! The model writes a passage with a glossary and questions. Before the learner
//! sees it, local checks decide whether it is usable: length for the level,
//! vocabulary profile, glossary words that occur in the passage, and valid
//! questions. A failing text is regenerated once; after that the learner is
//! offered authored reading sets from units. The checks cover form, not truth,
//! so every generated text is labelled generated. Nothing here counts toward an
//! estimate.

mod check;
mod prompt;
mod service;

use curriculum::{Activity, Question, Unit};
use serde::{Deserialize, Serialize};

pub use check::{ReadingProblem, check_reading};
pub use prompt::{
    READING_PASSAGE_VERSION, ReadingSpec, USER_MESSAGE as READING_USER_MESSAGE,
    system_prompt as reading_system_prompt,
};
pub use service::{
    GeneratedReading, QuestionResult, ReadingConfig, ReadingDeps, ReadingFallback,
    ReadingFallbackReason, ReadingOutcome, ReadingScore, ReadingSession, ReadingTopicChoice,
    StoredReading, list_generated_readings,
};

/// A glossary entry: the word as it appears in the passage, a gloss in the
/// learner's first language and a new example sentence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlossaryEntry {
    pub word: String,
    pub gloss_l1: String,
    pub example: String,
}

/// A multiple-choice question about the passage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadingQuestion {
    pub stem: String,
    pub options: Vec<String>,
    pub answer_index: i64,
    pub explanation_en: String,
    pub explanation_l1: String,
}

/// The reply of the contract, typed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawReading {
    pub title: String,
    pub passage: String,
    pub glossary: Vec<GlossaryEntry>,
    pub questions: Vec<ReadingQuestion>,
}

/// An authored reading set offered when generation is not possible.
#[derive(Debug, Clone, PartialEq)]
pub struct AuthoredReading {
    pub unit_id: String,
    pub activity_id: String,
    pub title: String,
    pub passage: String,
    pub questions: Vec<Question>,
}

/// The authored `reading_set` activities of the units at `level`, in unit order.
pub fn authored_reading_sets(
    units: &[Unit],
    level: assessment_engine::Level,
) -> Vec<AuthoredReading> {
    let wanted = crate::support::curriculum_level(level);
    let mut sets = Vec::new();
    for unit in units.iter().filter(|u| u.level == wanted) {
        for activity in &unit.activities {
            if let Activity::ReadingSet(set) = activity {
                sets.push(AuthoredReading {
                    unit_id: unit.id.clone(),
                    activity_id: set.id.clone(),
                    title: unit.title.en.clone(),
                    passage: set.passage.clone(),
                    questions: set.questions.clone(),
                });
            }
        }
    }
    sets
}
