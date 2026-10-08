//! Activities of a unit session: what the learner is shown and what they hand in.
//! Audio never travels over this API: a listening item carries the text for the
//! server's speech output, and a recording is made at the server's microphone.

use curriculum::{ActivityType, Localized, Skill};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::feedback::ActivityResultView;

/// One line for speech output. A dialogue has a speaker per line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AudioLineView {
    pub speaker: Option<String>,
    pub text: String,
}

/// A question of a reading or listening set, without its answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct QuestionPrompt {
    pub stem: String,
    pub options: Vec<String>,
}

/// One item of a listening minimal-pairs activity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PairChoice {
    pub options: Vec<String>,
    pub audio: AudioLineView,
}

/// One pair of a say-mode minimal-pairs drill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PairPrompt {
    pub a: String,
    pub b: String,
    pub focus: String,
}

/// The content of an activity by type, with every answer left out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum ActivityBody {
    Mcq {
        passage: Option<String>,
        /// A listening item: played by the server, not shown.
        audio: Option<Vec<AudioLineView>>,
        stem: String,
        options: Vec<String>,
    },
    GapFill {
        text: String,
        gaps: u32,
    },
    Reorder {
        tokens: Vec<String>,
    },
    Match {
        left: Vec<String>,
        /// The right-hand column in the order shown. A pick names a position in it.
        right: Vec<String>,
    },
    Dictation {
        audio: Vec<AudioLineView>,
    },
    MinimalPairsListen {
        items: Vec<PairChoice>,
    },
    MinimalPairsSay {
        pairs: Vec<PairPrompt>,
    },
    ReadingSet {
        passage: String,
        questions: Vec<QuestionPrompt>,
    },
    ListeningSet {
        audio: Vec<AudioLineView>,
        replays_allowed: u32,
        questions: Vec<QuestionPrompt>,
    },
    ErrorCorrection {
        sentence: String,
    },
    ReadAloud {
        text: String,
        focus_phonemes: Vec<String>,
    },
    Shadowing {
        title: String,
        lines: Vec<AudioLineView>,
    },
    Production {
        prompt: Localized,
        content_points: Vec<String>,
        min_words: u32,
        max_words: u32,
    },
    Mediation {
        source_text: String,
        task: Localized,
    },
    Roleplay {
        scenario: Localized,
        tutor_role: String,
        learner_role: String,
        goals: Vec<String>,
        max_turns: u32,
    },
}

/// What the learner is shown for one activity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ActivityPresentation {
    pub id: String,
    pub activity_type: ActivityType,
    pub skill: Skill,
    pub instructions: Localized,
    pub body: ActivityBody,
}

/// An activity of the run that cannot be done here, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ActivityUnavailable {
    pub activity_id: String,
    /// A sentence: what this program lacks to run the activity.
    pub reason: String,
}

/// `GET /api/sessions/{id}/next-activity`: the first activity of the run that has
/// no answer yet and can be done here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NextActivity {
    pub session_id: i64,
    /// Activities answered so far.
    pub answered: u32,
    /// Activities in this run.
    pub total: u32,
    /// `None` when every activity of the run is answered or cannot be done here.
    /// Stop the session to settle the checkpoint.
    pub activity: Option<ActivityPresentation>,
    /// Unanswered activities that cannot be done with what this program has, so
    /// the screen can say so instead of offering them.
    pub unavailable: Vec<ActivityUnavailable>,
}

/// What the learner hands in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum ActivityAnswer {
    /// The chosen option of a multiple-choice item.
    Choice { index: u32 },
    /// One answer per gap.
    Gaps { answers: Vec<String> },
    /// The tokens in the order the learner put them.
    Order { tokens: Vec<String> },
    /// A pick per item, `null` for an item left blank: the questions of a reading
    /// or listening set, the pairs of a match, the words of a listening minimal pair.
    Picks { picks: Vec<Option<u32>> },
    /// Typed text: a dictation, a corrected sentence, a written production.
    Text { text: String },
    /// The learner went through a practice drill that has nothing to score.
    Done,
    /// Start a roleplay activity: the tutor speaks first, the learner's lines go
    /// to `POST /api/sessions/{id}/text`.
    RoleplayStart,
    /// End the roleplay and score it when the activity has a rubric.
    RoleplayFinish,
}

/// `POST /api/activities/submit`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SubmitActivityRequest {
    pub session_id: i64,
    pub activity_id: String,
    pub answer: ActivityAnswer,
}

/// The answer to a submit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SubmitActivityResponse {
    /// `None` for `roleplay_start`, which has no result yet.
    pub result: Option<ActivityResultView>,
}
