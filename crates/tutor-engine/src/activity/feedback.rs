//! Feedback data for a scored activity.
//!
//! Data, not prose: which items were right, what was given, what was expected,
//! and the authored explanation that came with the content. Nothing here is
//! written by a model, and nothing states a level, a percentage or a verdict on
//! the learner. The command line and the UI decide how to word it.

use assessment_engine::is_success;
use curriculum::Localized;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemOutcome {
    Correct,
    /// One edit away from an accepted answer of five letters or more: half
    /// credit and a spelling note.
    Spelling,
    Wrong,
    /// Nothing was answered.
    Blank,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ItemFeedback {
    /// The gap, question, pair or word, counting from 0.
    pub index: usize,
    pub outcome: ItemOutcome,
    /// What the learner gave, as text.
    pub given: Option<String>,
    /// The accepted answer to show, the first one when there are several.
    pub expected: String,
    /// The authored explanation of this item, when the content has one.
    pub explanation: Option<Localized>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Feedback {
    /// 0 to 1.
    pub score: f64,
    /// Items answered fully right.
    pub correct: usize,
    pub total: usize,
    /// The score is at or above the success line of the spec.
    pub passed: bool,
    pub items: Vec<ItemFeedback>,
    /// The authored explanation of the activity as a whole.
    pub explanation: Option<Localized>,
}

impl Feedback {
    pub(crate) fn new(
        score: f64,
        items: Vec<ItemFeedback>,
        explanation: Option<Localized>,
    ) -> Self {
        let correct = items
            .iter()
            .filter(|i| i.outcome == ItemOutcome::Correct)
            .count();
        Self {
            score,
            correct,
            total: items.len(),
            passed: is_success(score),
            items,
            explanation,
        }
    }
}
