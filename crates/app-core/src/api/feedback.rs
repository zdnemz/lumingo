//! What the tutor tells the learner about their work: turn analysis, writing
//! feedback, activity results, reading scores and the end-of-session summary.
//!
//! None of these carries a level. A level appears only in the progress estimates,
//! which are computed from authored attempts.

use curriculum::{ActivityType, Localized};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::mirror::api_enum;

api_enum! {
    /// How much an error gets in the way.
    ErrorSeverity <=> storage::Severity {
        Minor => "minor",
        Major => "major",
        Blocking => "blocking",
    }
}

/// One error found in a learner turn. The quote is the learner's own words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ErrorFindingView {
    /// A code of the error catalog, such as `verb_tense`.
    pub category: String,
    pub quote: String,
    pub correction: String,
    pub severity: ErrorSeverity,
    /// The tutor's reply already worked on this error.
    pub addressed_in_reply: bool,
}

/// How an objective was shown in a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum EvidenceStatusView {
    Demonstrated,
    Partial,
    NotDemonstrated,
}

/// A can-do of the unit that the turn showed, or did not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ObjectiveEvidenceView {
    pub objective_id: String,
    pub status: EvidenceStatusView,
    pub quote: String,
}

/// The analysis of one learner turn (`AnalysisReady`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TurnAnalysisView {
    /// The position of the analysed turn in the stored session.
    pub turn_seq: i64,
    pub errors: Vec<ErrorFindingView>,
    pub objective_evidence: Vec<ObjectiveEvidenceView>,
    /// A short note the tutor keeps for the next turn.
    pub note: String,
    /// Entries a filter removed because their quote was not in the learner's words.
    pub dropped: u32,
}

/// One error category of the end summary, with one example.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ErrorPatternView {
    pub category: String,
    pub count: u32,
    pub quote: String,
    pub correction: String,
}

/// The end summary of a conversation: what to practise next.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ConversationSummaryView {
    pub learner_turns: u32,
    /// Most frequent first, at most five.
    pub top_errors: Vec<ErrorPatternView>,
    /// Positions of learner turns that have no analysis.
    pub unanalysed_turns: Vec<i64>,
    /// The filters removed a third or more of the analysis entries lately.
    pub analysis_unreliable: bool,
}

/// One pronunciation word.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PronWordView {
    pub word: String,
    /// 0 to 1, when the word was scored and a calibration curve exists.
    pub score: Option<f32>,
    /// Phonemes below the class threshold.
    pub flagged_phonemes: u32,
    /// The word was not looked up or could not be aligned.
    pub checked: bool,
}

/// Pronunciation findings for one recording (`PronFindings`). Always experimental.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PronFindingsView {
    /// The UI labels pronunciation feedback experimental.
    pub experimental: bool,
    pub utterance_score: Option<f32>,
    pub scores_calibrated: bool,
    pub words: Vec<PronWordView>,
    /// Indices into `words` of the words to look at first.
    pub highlighted: Vec<u32>,
}

/// The result of one item of a deterministic activity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ItemOutcomeView {
    Correct,
    /// One edit from an accepted answer: half credit and a spelling note.
    Spelling,
    Wrong,
    Blank,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ItemFeedbackView {
    pub index: u32,
    pub outcome: ItemOutcomeView,
    pub given: Option<String>,
    /// The accepted answer, the first one when there are several.
    pub expected: String,
    pub explanation: Option<Localized>,
}

/// Feedback on an objective activity: authored data, nothing a model wrote.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DeterministicFeedbackView {
    /// 0 to 1.
    pub score: f64,
    pub correct: u32,
    pub total: u32,
    pub passed: bool,
    pub items: Vec<ItemFeedbackView>,
    pub explanation: Option<Localized>,
}

/// One dimension of a rubric score.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RubricDimensionView {
    /// `task_achievement`, `range`, `accuracy`, `coherence` or `interaction`.
    pub dimension: String,
    /// 0 to 4, the mean over the runs. `None` while the dimension needs review.
    pub band: Option<f64>,
    pub reason: String,
    /// Quotes that are in the learner's response.
    pub evidence_quotes: Vec<String>,
    /// `scored` or `needs_review`.
    pub status: String,
    /// A cross-check lowered the band.
    pub capped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ContentPointView {
    pub point: String,
    pub covered: bool,
    pub quote: String,
}

/// A rubric score of a written or spoken production. Bands are per dimension;
/// no level is stated.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RubricFeedbackView {
    pub dimensions: Vec<RubricDimensionView>,
    pub content_points: Vec<ContentPointView>,
    /// False: the response did not attempt the task and the learner is asked to try again.
    pub on_task: bool,
    pub feedback_en: String,
    pub feedback_l1: String,
    /// 0.1 to 0.9.
    pub confidence: f64,
    pub runs: u32,
}

/// What happened to a response to an activity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum ActivityOutcomeView {
    Deterministic {
        feedback: DeterministicFeedbackView,
    },
    Rubric {
        feedback: RubricFeedbackView,
    },
    /// A productive response waiting for a provider. It is stored and scored later.
    Queued,
    Pron {
        findings: Vec<PronFindingsView>,
    },
    /// Stored with nothing to score. The reason is a sentence.
    Unscored {
        reason: String,
    },
}

/// The result of one activity (`FeedbackReady`, and the answer to a submit).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ActivityResultView {
    pub activity_id: String,
    pub activity_type: ActivityType,
    /// 0 to 1 when there is a score.
    pub score: Option<f64>,
    pub confidence: Option<f64>,
    pub outcome: ActivityOutcomeView,
}

/// One checkpoint activity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CheckpointRowView {
    pub activity_id: String,
    pub answered: bool,
    pub score: Option<f64>,
}

/// The end of a unit run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UnitSummaryView {
    pub unit_id: String,
    pub rows: Vec<CheckpointRowView>,
    /// Mean of the scored checkpoint items, 0 to 1.
    pub mean: f64,
    pub pass_mark: f64,
    pub passed: bool,
    /// The pass rests on objective items only and an item still waits for a
    /// provider, so the unit stays in progress.
    pub provisional: bool,
    pub unit_status: super::progress::UnitStatus,
    pub answered: u32,
}

/// An earlier error of a writing draft and what became of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DraftResolution {
    Fixed,
    Remaining,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DraftErrorView {
    pub quote: String,
    pub category: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EarlierErrorView {
    pub error: DraftErrorView,
    pub resolution: DraftResolution,
}

/// A revised draft against the one before: fixed, remaining and new errors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DraftComparisonView {
    pub earlier: Vec<EarlierErrorView>,
    pub new: Vec<DraftErrorView>,
}

/// How far the feedback of a draft has come.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DraftStatusView {
    /// Errors and, when there is a rubric, bands are in.
    Analysed,
    /// The errors are in. The bands wait for a provider.
    RubricPending,
    /// Nothing from a provider yet: the draft waits and is finished later.
    Pending,
}

/// Layers two and three of a writing draft's feedback (`FeedbackReady`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DraftFeedbackView {
    pub turn_seq: i64,
    pub status: DraftStatusView,
    pub analysis: Option<TurnAnalysisView>,
    pub comparison: Option<DraftComparisonView>,
    pub rubric: Option<RubricFeedbackView>,
}

/// What the learner chose for one question, and how it went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct QuestionResultView {
    pub chosen: Option<u32>,
    pub correct_index: u32,
    pub correct: bool,
    pub explanation_en: String,
    pub explanation_l1: String,
}

/// The score of a reading text's questions. Practice only: it never counts toward an estimate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ReadingScoreView {
    pub correct: u32,
    pub total: u32,
    /// 0 to 1.
    pub score: f64,
    pub questions: Vec<QuestionResultView>,
}

/// Feedback the server pushes as an event, by what it is about.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum FeedbackView {
    /// The end summary of a conversation, text or voice.
    Conversation { summary: ConversationSummaryView },
    /// The end of a lesson, checkpoint or drill run.
    Unit { summary: UnitSummaryView },
    /// The feedback of one writing draft.
    Draft { feedback: DraftFeedbackView },
    /// The result of one activity.
    Activity { result: ActivityResultView },
    /// The score of a reading text's questions.
    Reading { score: ReadingScoreView },
}
