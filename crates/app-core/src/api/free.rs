//! The free modes: the writing workshop and graded reading. Both are practice
//! only: their attempts are stored as free-mode work and never count toward an
//! estimate.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::feedback::ReadingScoreView;
use super::sessions::TopicChoice;

/// `POST /api/writing/{session}/drafts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SubmitDraftRequest {
    pub text: String,
}

/// Layer one of a draft's feedback, which needs no provider and comes at once.
/// Layers two and three arrive as a `FeedbackReady` event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DraftAccepted {
    pub session_id: i64,
    /// The position of the draft in the stored session.
    pub turn_seq: i64,
    pub words: u32,
    /// False when no rule-based checker is built in: `rule_findings` is then
    /// empty because nothing was checked, not because the text is clean.
    pub rule_checked: bool,
    pub rule_findings: Vec<String>,
    /// The draft is shorter than the task's minimum.
    pub below_minimum: bool,
}

/// `POST /api/reading/generate`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GenerateReadingRequest {
    /// A session of kind `reading`.
    pub session_id: i64,
    pub topic: TopicChoice,
}

/// A word of the passage with its gloss.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GlossEntryView {
    pub word: String,
    pub gloss_l1: String,
    pub example: String,
}

/// A question as shown before it is answered: no answer and no explanation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ReadingQuestionView {
    pub stem: String,
    pub options: Vec<String>,
}

/// A reading text and its questions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ReadingTextView {
    pub title: String,
    pub passage: String,
    pub glossary: Vec<GlossEntryView>,
    pub questions: Vec<ReadingQuestionView>,
}

/// Where an authored reading set that is offered in place of a generated text lives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AuthoredSetView {
    pub unit_id: String,
    pub activity_id: String,
    pub text: ReadingTextView,
}

/// Why authored sets are offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ReadingFallbackReasonView {
    ProviderUnavailable,
    /// The model's text failed a check twice.
    Unusable,
}

/// The answer to `POST /api/reading/generate`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum ReadingOutcomeView {
    /// A generated text that passed the local checks. It is labelled generated:
    /// the checks cover form, not truth.
    Generated {
        content_id: i64,
        text: ReadingTextView,
        regenerated: bool,
        /// False when no word list was available for the vocabulary check.
        vocabulary_checked: bool,
    },
    /// Authored sets of the learner's level, in place of a text.
    Fallback {
        reason: ReadingFallbackReasonView,
        sets: Vec<AuthoredSetView>,
    },
}

/// Which text the answers are for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum ReadingTarget {
    Generated {
        content_id: i64,
    },
    Authored {
        unit_id: String,
        activity_id: String,
    },
}

/// `POST /api/reading/{session}/answers`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AnswerReadingRequest {
    pub target: ReadingTarget,
    /// One entry per question, `null` for one left open.
    pub answers: Vec<Option<u32>>,
}

/// The score, with the explanations that were held back until now.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ReadingAnswered {
    pub score: ReadingScoreView,
}
