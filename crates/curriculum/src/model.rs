//! Rust types that mirror `curriculum/schema/unit.schema.json` field for field.
//!
//! Every object the schema closes with `additionalProperties: false` or
//! `unevaluatedProperties: false` carries `deny_unknown_fields` here, so a field the
//! schema does not know cannot slip in through serde either. Value limits (lengths,
//! ranges, patterns) stay in the schema: the loader runs the schema first and only
//! deserialises a document that passed it.

use serde::{Deserialize, Serialize};

/// CEFR level of a unit. The order is the learning order, so `A1 < C2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub enum Level {
    A1,
    A2,
    B1,
    B2,
    C1,
    C2,
}

impl Level {
    /// Every level in learning order.
    pub const ALL: [Level; 6] = [
        Level::A1,
        Level::A2,
        Level::B1,
        Level::B2,
        Level::C1,
        Level::C2,
    ];

    /// Upper-case name as written in unit files, for example `A1`.
    pub fn as_str(self) -> &'static str {
        match self {
            Level::A1 => "A1",
            Level::A2 => "A2",
            Level::B1 => "B1",
            Level::B2 => "B2",
            Level::C1 => "C1",
            Level::C2 => "C2",
        }
    }

    /// Lower-case name as used in unit ids and folder names, for example `a1`.
    pub fn lowercase(self) -> &'static str {
        match self {
            Level::A1 => "a1",
            Level::A2 => "a2",
            Level::B1 => "b1",
            Level::B2 => "b2",
            Level::C1 => "c1",
            Level::C2 => "c2",
        }
    }

    /// Parses `A1` to `C2`, in either case.
    pub fn parse(text: &str) -> Option<Level> {
        Level::ALL
            .into_iter()
            .find(|level| level.as_str().eq_ignore_ascii_case(text))
    }
}

impl std::fmt::Display for Level {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Word-list level of a vocabulary item. `Unlisted` means no word list has the word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub enum LevelTag {
    A1,
    A2,
    B1,
    B2,
    C1,
    C2,
    #[serde(rename = "unlisted")]
    Unlisted,
}

impl LevelTag {
    /// The CEFR level, or `None` for an unlisted word.
    pub fn level(self) -> Option<Level> {
        match self {
            LevelTag::A1 => Some(Level::A1),
            LevelTag::A2 => Some(Level::A2),
            LevelTag::B1 => Some(Level::B1),
            LevelTag::B2 => Some(Level::B2),
            LevelTag::C1 => Some(Level::C1),
            LevelTag::C2 => Some(Level::C2),
            LevelTag::Unlisted => None,
        }
    }
}

impl From<Level> for LevelTag {
    fn from(level: Level) -> Self {
        match level {
            Level::A1 => LevelTag::A1,
            Level::A2 => LevelTag::A2,
            Level::B1 => LevelTag::B1,
            Level::B2 => LevelTag::B2,
            Level::C1 => LevelTag::C1,
            Level::C2 => LevelTag::C2,
        }
    }
}

/// Skill an objective or an activity trains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub enum Skill {
    Listening,
    Reading,
    SpeakingProduction,
    SpeakingInteraction,
    Writing,
    Mediation,
    Grammar,
    Vocabulary,
    Pronunciation,
}

/// English text plus optional Indonesian. The validators require `id` for levels A1 to B1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Localized {
    pub en: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub id: Option<String>,
}

/// One authored unit of about 60 learning minutes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Unit {
    /// Always `1.0` for this schema.
    pub schema_version: String,
    pub id: String,
    pub level: Level,
    /// Position inside the level, 1 to 30.
    pub sequence: u8,
    pub title: Localized,
    pub theme: String,
    pub estimated_minutes: u32,
    pub prerequisites: Vec<String>,
    pub objectives: Vec<Objective>,
    pub targets: Targets,
    pub presentation: Vec<PresentationBlock>,
    pub dialogues: Vec<Dialogue>,
    pub activities: Vec<Activity>,
    pub checkpoint: Checkpoint,
    pub generation_policy: GenerationPolicy,
    pub review_items: Vec<ReviewItem>,
    pub provenance: Provenance,
}

/// An observable can-do statement tied to a skill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Objective {
    pub id: String,
    pub skill: Skill,
    pub can_do: Localized,
    /// Name of a CEFR scale. A reference only, never descriptor text.
    pub cefr_scale_ref: String,
}

/// What the unit teaches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Targets {
    pub vocabulary: Vec<VocabItem>,
    pub grammar: Vec<GrammarPoint>,
    pub functions: Vec<String>,
    pub pronunciation: Vec<PronFocus>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub enum PartOfSpeech {
    Noun,
    Verb,
    Adjective,
    Adverb,
    Pronoun,
    Preposition,
    Determiner,
    Conjunction,
    Interjection,
    Phrase,
}

/// Indonesian gloss of a vocabulary item. The schema has no English field here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Gloss {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct VocabItem {
    pub id: String,
    pub lemma: String,
    pub pos: PartOfSpeech,
    pub level_tag: LevelTag,
    pub gloss: Gloss,
    pub example: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct GrammarPoint {
    pub id: String,
    pub name: String,
    pub pattern: String,
    pub note: Localized,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct PronFocus {
    pub id: String,
    pub focus: String,
    /// ARPAbet symbols without stress digits, for example `TH`.
    pub phonemes: Vec<String>,
    pub examples: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub enum PresentationKind {
    Explanation,
    Tip,
    CultureNote,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct PresentationBlock {
    pub id: String,
    pub kind: PresentationKind,
    pub text: Localized,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub examples: Option<Vec<Localized>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct DialogueTurn {
    pub speaker: String,
    pub text: String,
}

/// A model dialogue, used for listening and shadowing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Dialogue {
    pub id: String,
    pub title: String,
    pub context: Localized,
    pub turns: Vec<DialogueTurn>,
}

/// How an activity is scored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub enum Scoring {
    Deterministic,
    Rubric,
    Pron,
    None,
}

/// Quality band of a model answer. The three bands double as rubric scorer anchors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub enum Band {
    Below,
    At,
    Above,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ModelAnswer {
    pub band: Band,
    pub text: String,
}

/// A multiple-choice question inside a `reading_set` or `listening_set`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Question {
    pub stem: String,
    pub options: Vec<String>,
    pub answer_index: u8,
    pub explanation: Localized,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct MatchPair {
    pub left: String,
    pub right: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct MinimalPair {
    pub a: String,
    pub b: String,
    pub focus: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub enum MinimalPairsMode {
    ListenChoose,
    SayBoth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub enum RoleplayMode {
    Fluency,
    Accuracy,
}

// The schema builds every activity from `activity_common` plus its own properties and
// closes it with `unevaluatedProperties: false`. serde cannot combine `flatten` with
// `deny_unknown_fields`, so this macro writes the common fields into each struct.
macro_rules! activity_struct {
    (
        $(#[$meta:meta])*
        $name:ident {
            $( $(#[$field_meta:meta])* $field:ident : $ty:ty ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        #[cfg_attr(feature = "ts", derive(ts_rs::TS))]
        #[cfg_attr(feature = "ts", ts(export))]
        pub struct $name {
            pub id: String,
            pub skill: Skill,
            pub objective_ids: Vec<String>,
            pub instructions: Localized,
            pub scoring: Scoring,
            $( $(#[$field_meta])* pub $field: $ty, )*
        }
    };
}

activity_struct! {
    /// Multiple choice with exactly one correct option.
    /// A `passage` makes it a reading item and an `audio_text` makes it a listening item.
    Mcq {
        /// Reading text shown above the stem.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        passage: Option<String>,
        /// Text spoken by TTS and not shown.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        audio_text: Option<String>,
        stem: String,
        options: Vec<String>,
        answer_index: u8,
        explanation: Localized,
    }
}

activity_struct! {
    /// A text with `___` gaps and the accepted answers of each gap.
    GapFill {
        text: String,
        /// One inner list of accepted answers per gap, in order.
        answers: Vec<Vec<String>>,
        explanation: Localized,
    }
}

activity_struct! {
    /// Tokens to put in order.
    Reorder {
        tokens: Vec<String>,
        answer: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        explanation: Option<Localized>,
    }
}

activity_struct! {
    /// Pairs to match.
    Match {
        pairs: Vec<MatchPair>,
    }
}

activity_struct! {
    /// Listen and type what was said.
    Dictation {
        audio_text: String,
        accepted_answers: Vec<String>,
    }
}

activity_struct! {
    /// Read a known text aloud. Scored by the pronunciation engine.
    ReadAloud {
        text: String,
        focus_phonemes: Vec<String>,
    }
}

activity_struct! {
    /// Minimal pairs, heard or said.
    MinimalPairs {
        mode: MinimalPairsMode,
        pairs: Vec<MinimalPair>,
    }
}

activity_struct! {
    /// Listen to each line of a dialogue and repeat it.
    Shadowing {
        dialogue_id: String,
    }
}

activity_struct! {
    /// A prompted spoken or written production with three model answers.
    /// `guided_speaking` and `guided_writing` share this shape.
    GuidedProduction {
        prompt: Localized,
        content_points: Vec<String>,
        rubric_id: String,
        /// Exactly one answer per band.
        model_answers: Vec<ModelAnswer>,
        min_words: u32,
        max_words: u32,
    }
}

activity_struct! {
    /// A conversation with the tutor in a role.
    Roleplay {
        scenario: Localized,
        tutor_role: String,
        learner_role: String,
        goals: Vec<String>,
        target_grammar_ids: Vec<String>,
        target_vocab_ids: Vec<String>,
        max_turns: u8,
        mode: RoleplayMode,
    }
}

activity_struct! {
    /// Relay or reshape a source text for a purpose.
    Mediation {
        source_text: String,
        task: Localized,
        rubric_id: String,
        model_answers: Vec<ModelAnswer>,
    }
}

activity_struct! {
    /// A passage with several questions.
    ReadingSet {
        passage: String,
        questions: Vec<Question>,
    }
}

activity_struct! {
    /// TTS audio with several questions. Exactly one of `audio_text` and `dialogue_id` is present.
    ListeningSet {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        audio_text: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        dialogue_id: Option<String>,
        replays_allowed: u8,
        questions: Vec<Question>,
    }
}

activity_struct! {
    /// Rewrite a sentence that has exactly one mistake.
    ErrorCorrection {
        sentence: String,
        accepted_answers: Vec<String>,
        error_category: String,
        explanation: Localized,
    }
}

/// Every activity type of the schema, tagged by `type`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub enum Activity {
    Mcq(Mcq),
    GapFill(GapFill),
    Reorder(Reorder),
    Match(Match),
    Dictation(Dictation),
    ReadAloud(ReadAloud),
    MinimalPairs(MinimalPairs),
    Shadowing(Shadowing),
    GuidedSpeaking(GuidedProduction),
    GuidedWriting(GuidedProduction),
    Roleplay(Roleplay),
    Mediation(Mediation),
    ReadingSet(ReadingSet),
    ListeningSet(ListeningSet),
    ErrorCorrection(ErrorCorrection),
}

/// The activity type names as they appear in the `type` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub enum ActivityType {
    Mcq,
    GapFill,
    Reorder,
    Match,
    Dictation,
    ReadAloud,
    MinimalPairs,
    Shadowing,
    GuidedSpeaking,
    GuidedWriting,
    Roleplay,
    Mediation,
    ReadingSet,
    ListeningSet,
    ErrorCorrection,
}

impl ActivityType {
    /// The value of the `type` field.
    pub fn as_str(self) -> &'static str {
        match self {
            ActivityType::Mcq => "mcq",
            ActivityType::GapFill => "gap_fill",
            ActivityType::Reorder => "reorder",
            ActivityType::Match => "match",
            ActivityType::Dictation => "dictation",
            ActivityType::ReadAloud => "read_aloud",
            ActivityType::MinimalPairs => "minimal_pairs",
            ActivityType::Shadowing => "shadowing",
            ActivityType::GuidedSpeaking => "guided_speaking",
            ActivityType::GuidedWriting => "guided_writing",
            ActivityType::Roleplay => "roleplay",
            ActivityType::Mediation => "mediation",
            ActivityType::ReadingSet => "reading_set",
            ActivityType::ListeningSet => "listening_set",
            ActivityType::ErrorCorrection => "error_correction",
        }
    }
}

/// The fields every activity has, borrowed from whichever variant it is.
#[derive(Debug, Clone, Copy)]
pub struct ActivityCommon<'a> {
    pub id: &'a str,
    pub activity_type: ActivityType,
    pub skill: Skill,
    pub objective_ids: &'a [String],
    pub instructions: &'a Localized,
    pub scoring: Scoring,
}

impl Activity {
    /// The common fields, whatever the variant.
    pub fn common(&self) -> ActivityCommon<'_> {
        macro_rules! common {
            ($a:expr, $t:expr) => {
                ActivityCommon {
                    id: &$a.id,
                    activity_type: $t,
                    skill: $a.skill,
                    objective_ids: &$a.objective_ids,
                    instructions: &$a.instructions,
                    scoring: $a.scoring,
                }
            };
        }
        match self {
            Activity::Mcq(a) => common!(a, ActivityType::Mcq),
            Activity::GapFill(a) => common!(a, ActivityType::GapFill),
            Activity::Reorder(a) => common!(a, ActivityType::Reorder),
            Activity::Match(a) => common!(a, ActivityType::Match),
            Activity::Dictation(a) => common!(a, ActivityType::Dictation),
            Activity::ReadAloud(a) => common!(a, ActivityType::ReadAloud),
            Activity::MinimalPairs(a) => common!(a, ActivityType::MinimalPairs),
            Activity::Shadowing(a) => common!(a, ActivityType::Shadowing),
            Activity::GuidedSpeaking(a) => common!(a, ActivityType::GuidedSpeaking),
            Activity::GuidedWriting(a) => common!(a, ActivityType::GuidedWriting),
            Activity::Roleplay(a) => common!(a, ActivityType::Roleplay),
            Activity::Mediation(a) => common!(a, ActivityType::Mediation),
            Activity::ReadingSet(a) => common!(a, ActivityType::ReadingSet),
            Activity::ListeningSet(a) => common!(a, ActivityType::ListeningSet),
            Activity::ErrorCorrection(a) => common!(a, ActivityType::ErrorCorrection),
        }
    }

    pub fn id(&self) -> &str {
        self.common().id
    }

    pub fn activity_type(&self) -> ActivityType {
        self.common().activity_type
    }

    pub fn skill(&self) -> Skill {
        self.common().skill
    }
}

/// The scored subset of activities that decides whether the unit is passed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Checkpoint {
    pub pass_score: f64,
    pub activity_ids: Vec<String>,
}

/// Item types the runtime model may generate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub enum GeneratedType {
    Mcq,
    GapFill,
    Reorder,
}

/// Limits for extra practice generated at run time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct GenerationPolicy {
    pub allowed_types: Vec<GeneratedType>,
    pub max_items_per_session: u8,
    pub max_level: Level,
    pub allowed_grammar_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub enum ReviewKind {
    Vocab,
    Grammar,
    Pron,
}

/// A target that enters spaced repetition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ReviewItem {
    pub kind: ReviewKind,
    pub r#ref: String,
}

/// How far a unit was checked. Only the owner sets the two human values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub enum ReviewStatus {
    Unreviewed,
    CriticPassed,
    HumanSampled,
    HumanFull,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Provenance {
    pub authored_by: String,
    pub authoring_model: String,
    pub review_status: ReviewStatus,
    /// Semantic version of this unit's content.
    pub content_version: String,
    pub sources: Vec<String>,
    /// Date of the last change, `YYYY-MM-DD`.
    pub updated: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub notes: Option<String>,
}
