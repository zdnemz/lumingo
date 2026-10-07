//! Rust types that mirror `curriculum/schema/unit.schema.json` (schema_version 1.0).
//! The schema is the authority: a unit is validated against it before it is
//! turned into these types, so deserialising here only ever sees valid shapes.

use serde::{Deserialize, Serialize};

pub type UnitId = String;
pub type LocalId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Level {
    A1,
    A2,
    B1,
    B2,
    C1,
    C2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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

impl Skill {
    /// The exact text the JSON form uses, and what the database index stores.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Listening => "listening",
            Self::Reading => "reading",
            Self::SpeakingProduction => "speaking_production",
            Self::SpeakingInteraction => "speaking_interaction",
            Self::Writing => "writing",
            Self::Mediation => "mediation",
            Self::Grammar => "grammar",
            Self::Vocabulary => "vocabulary",
            Self::Pronunciation => "pronunciation",
        }
    }
}

/// English is always present. Indonesian is required for A1 to B1 by the validators.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Localized {
    pub en: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Objective {
    pub id: LocalId,
    pub skill: Skill,
    pub can_do: Localized,
    pub cefr_scale_ref: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdGloss {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VocabItem {
    pub id: LocalId,
    pub lemma: String,
    pub pos: PartOfSpeech,
    pub level_tag: LevelTag,
    pub gloss: IdGloss,
    pub example: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrammarPoint {
    pub id: LocalId,
    pub name: String,
    pub pattern: String,
    pub note: Localized,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PronFocus {
    pub id: LocalId,
    pub focus: String,
    pub phonemes: Vec<String>,
    pub examples: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Targets {
    pub vocabulary: Vec<VocabItem>,
    pub grammar: Vec<GrammarPoint>,
    pub functions: Vec<String>,
    pub pronunciation: Vec<PronFocus>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresentationKind {
    Explanation,
    Tip,
    CultureNote,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentationBlock {
    pub id: LocalId,
    pub kind: PresentationKind,
    pub text: Localized,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<Localized>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DialogueTurn {
    pub speaker: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dialogue {
    pub id: LocalId,
    pub title: String,
    pub context: Localized,
    pub turns: Vec<DialogueTurn>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scoring {
    Deterministic,
    Rubric,
    Pron,
    None,
}

/// Fields every activity carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Common {
    pub id: LocalId,
    pub skill: Skill,
    pub objective_ids: Vec<LocalId>,
    pub instructions: Localized,
    /// Optional in the schema: each type fixes its own default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scoring: Option<Scoring>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelBand {
    Below,
    At,
    Above,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelAnswer {
    pub band: ModelBand,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    pub stem: String,
    pub options: Vec<String>,
    pub answer_index: u8,
    pub explanation: Localized,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchPair {
    pub left: String,
    pub right: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MinimalPair {
    pub a: String,
    pub b: String,
    pub focus: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MinimalPairsMode {
    ListenChoose,
    SayBoth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleplayMode {
    Fluency,
    Accuracy,
}

/// The fields of `act_guided_production`, shared by its two wire names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuidedProduction {
    #[serde(flatten)]
    pub common: Common,
    pub prompt: Localized,
    pub content_points: Vec<String>,
    pub rubric_id: LocalId,
    pub model_answers: Vec<ModelAnswer>,
    pub min_words: u32,
    pub max_words: u32,
}

/// One activity. `type` selects the variant, as in the schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Activity {
    Mcq {
        #[serde(flatten)]
        common: Common,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        passage: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        audio_text: Option<String>,
        stem: String,
        options: Vec<String>,
        answer_index: u8,
        explanation: Localized,
    },
    GapFill {
        #[serde(flatten)]
        common: Common,
        text: String,
        answers: Vec<Vec<String>>,
        explanation: Localized,
    },
    Reorder {
        #[serde(flatten)]
        common: Common,
        tokens: Vec<String>,
        answer: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        explanation: Option<Localized>,
    },
    Match {
        #[serde(flatten)]
        common: Common,
        pairs: Vec<MatchPair>,
    },
    Dictation {
        #[serde(flatten)]
        common: Common,
        audio_text: String,
        accepted_answers: Vec<String>,
    },
    ReadAloud {
        #[serde(flatten)]
        common: Common,
        text: String,
        focus_phonemes: Vec<String>,
    },
    MinimalPairs {
        #[serde(flatten)]
        common: Common,
        mode: MinimalPairsMode,
        pairs: Vec<MinimalPair>,
    },
    Shadowing {
        #[serde(flatten)]
        common: Common,
        dialogue_id: LocalId,
    },
    GuidedSpeaking(GuidedProduction),
    GuidedWriting(GuidedProduction),
    Roleplay {
        #[serde(flatten)]
        common: Common,
        scenario: Localized,
        tutor_role: String,
        learner_role: String,
        goals: Vec<String>,
        target_grammar_ids: Vec<LocalId>,
        target_vocab_ids: Vec<LocalId>,
        max_turns: u32,
        mode: RoleplayMode,
    },
    Mediation {
        #[serde(flatten)]
        common: Common,
        source_text: String,
        task: Localized,
        rubric_id: LocalId,
        model_answers: Vec<ModelAnswer>,
    },
    ReadingSet {
        #[serde(flatten)]
        common: Common,
        passage: String,
        questions: Vec<Question>,
    },
    ListeningSet {
        #[serde(flatten)]
        common: Common,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        audio_text: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dialogue_id: Option<LocalId>,
        replays_allowed: u32,
        questions: Vec<Question>,
    },
    ErrorCorrection {
        #[serde(flatten)]
        common: Common,
        sentence: String,
        accepted_answers: Vec<String>,
        error_category: String,
        explanation: Localized,
    },
}

impl Activity {
    pub fn common(&self) -> &Common {
        match self {
            Self::GuidedSpeaking(g) | Self::GuidedWriting(g) => &g.common,
            Self::Mcq { common, .. }
            | Self::GapFill { common, .. }
            | Self::Reorder { common, .. }
            | Self::Match { common, .. }
            | Self::Dictation { common, .. }
            | Self::ReadAloud { common, .. }
            | Self::MinimalPairs { common, .. }
            | Self::Shadowing { common, .. }
            | Self::Roleplay { common, .. }
            | Self::Mediation { common, .. }
            | Self::ReadingSet { common, .. }
            | Self::ListeningSet { common, .. }
            | Self::ErrorCorrection { common, .. } => common,
        }
    }

    /// The `type` text of the unit format, exactly as the schema writes it.
    /// The attempt rows store this as `activity_type`.
    pub const fn type_str(&self) -> &'static str {
        match self {
            Self::Mcq { .. } => "mcq",
            Self::GapFill { .. } => "gap_fill",
            Self::Reorder { .. } => "reorder",
            Self::Match { .. } => "match",
            Self::Dictation { .. } => "dictation",
            Self::ReadAloud { .. } => "read_aloud",
            Self::MinimalPairs { .. } => "minimal_pairs",
            Self::Shadowing { .. } => "shadowing",
            Self::GuidedSpeaking(_) => "guided_speaking",
            Self::GuidedWriting(_) => "guided_writing",
            Self::Roleplay { .. } => "roleplay",
            Self::Mediation { .. } => "mediation",
            Self::ReadingSet { .. } => "reading_set",
            Self::ListeningSet { .. } => "listening_set",
            Self::ErrorCorrection { .. } => "error_correction",
        }
    }

    /// The dimension an attempt for this activity is filed under
    /// (ASSESSMENT_SPEC section 2). An `mcq` counts by what it exercises: audio
    /// makes it listening, a passage makes it reading, and otherwise the
    /// authored skill decides between listening, reading, vocabulary and
    /// grammar. `spoken` tells a `mediation` how it was answered; every other
    /// type ignores it. Grammar, vocabulary and pronunciation are the
    /// supporting dimensions: they are shown as mastery and never with a CEFR
    /// label.
    pub fn evidence_skill(&self, spoken: bool) -> &'static str {
        match self {
            Self::Mcq {
                audio_text,
                passage,
                common,
                ..
            } => {
                if audio_text.is_some() {
                    "listening"
                } else if passage.is_some() {
                    "reading"
                } else {
                    match common.skill {
                        Skill::Listening => "listening",
                        Skill::Reading => "reading",
                        Skill::Vocabulary => "vocabulary",
                        _ => "grammar",
                    }
                }
            }
            Self::GapFill { .. } | Self::Reorder { .. } => "grammar",
            Self::Match { .. } => "vocabulary",
            Self::Dictation { .. } | Self::ListeningSet { .. } => "listening",
            Self::MinimalPairs { mode, .. } => match mode {
                MinimalPairsMode::ListenChoose => "listening",
                MinimalPairsMode::SayBoth => "pronunciation",
            },
            Self::ReadingSet { .. } => "reading",
            Self::ErrorCorrection { .. } | Self::GuidedWriting(_) => "writing",
            Self::GuidedSpeaking(_) | Self::Roleplay { .. } => "speaking",
            Self::Mediation { .. } => {
                if spoken {
                    "speaking"
                } else {
                    "writing"
                }
            }
            Self::ReadAloud { .. } | Self::Shadowing { .. } => "pronunciation",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Checkpoint {
    pub pass_score: f64,
    pub activity_ids: Vec<LocalId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeneratedType {
    Mcq,
    GapFill,
    Reorder,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationPolicy {
    pub allowed_types: Vec<GeneratedType>,
    pub max_items_per_session: u32,
    pub max_level: Level,
    pub allowed_grammar_ids: Vec<LocalId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewKind {
    Vocab,
    Grammar,
    Pron,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewItemRef {
    pub kind: ReviewKind,
    #[serde(rename = "ref")]
    pub target: LocalId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    Unreviewed,
    CriticPassed,
    HumanSampled,
    HumanFull,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub authored_by: String,
    pub authoring_model: String,
    pub review_status: ReviewStatus,
    pub content_version: String,
    pub sources: Vec<String>,
    pub updated: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Unit {
    pub schema_version: String,
    pub id: UnitId,
    pub level: Level,
    pub sequence: u32,
    pub title: Localized,
    pub theme: String,
    pub estimated_minutes: u32,
    pub prerequisites: Vec<UnitId>,
    pub objectives: Vec<Objective>,
    pub targets: Targets,
    pub presentation: Vec<PresentationBlock>,
    pub dialogues: Vec<Dialogue>,
    pub activities: Vec<Activity>,
    pub checkpoint: Checkpoint,
    pub generation_policy: GenerationPolicy,
    pub review_items: Vec<ReviewItemRef>,
    pub provenance: Provenance,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::UnitLoader;

    const EXAMPLE: &str = include_str!("../../../curriculum/examples/a1-u01.example.json");

    // Helper for the tests below; clippy.toml only exempts `#[test]` bodies.
    #[allow(clippy::unwrap_used)]
    fn example() -> Unit {
        UnitLoader::new().load_str(EXAMPLE).unwrap()
    }

    #[allow(clippy::unwrap_used)]
    fn activity(id: &str) -> Activity {
        example()
            .activities
            .into_iter()
            .find(|a| a.common().id == id)
            .unwrap()
    }

    #[test]
    fn every_activity_type_of_the_example_unit_names_its_schema_text() {
        let unit = example();
        let named: Vec<&str> = unit.activities.iter().map(Activity::type_str).collect();
        // The fifteen schema types, as the example unit uses them.
        assert_eq!(
            named,
            [
                "mcq",
                "mcq",
                "gap_fill",
                "reorder",
                "match",
                "dictation",
                "read_aloud",
                "minimal_pairs",
                "shadowing",
                "guided_speaking",
                "roleplay",
                "guided_writing",
                "mcq",
                "reading_set",
                "listening_set",
                "error_correction",
                "error_correction",
            ]
        );
        // The two wire names of guided production are distinct.
        assert_ne!(
            Activity::GuidedSpeaking(GuidedProduction {
                common: Common {
                    id: "x".to_owned(),
                    skill: Skill::SpeakingProduction,
                    objective_ids: Vec::new(),
                    instructions: Localized {
                        en: String::new(),
                        id: None,
                    },
                    scoring: Some(Scoring::Rubric),
                },
                prompt: Localized {
                    en: String::new(),
                    id: None,
                },
                content_points: Vec::new(),
                rubric_id: "r".to_owned(),
                model_answers: Vec::new(),
                min_words: 1,
                max_words: 2,
            })
            .type_str(),
            "guided_writing"
        );
    }

    #[test]
    fn the_evidence_dimension_follows_the_spec_table() {
        // An mcq counts by what it exercises.
        assert_eq!(
            activity("a01-listen-question").evidence_skill(false),
            "listening"
        );
        assert_eq!(activity("a13-read-budi").evidence_skill(false), "reading");
        assert_eq!(
            activity("a02-greeting-by-time").evidence_skill(false),
            "vocabulary"
        );
        // The spec's table, one row each.
        assert_eq!(activity("a03-gap-am").evidence_skill(false), "grammar");
        assert_eq!(
            activity("a04-reorder-name").evidence_skill(false),
            "grammar"
        );
        assert_eq!(
            activity("a05-match-phrases").evidence_skill(false),
            "vocabulary"
        );
        assert_eq!(
            activity("a06-dictation-from").evidence_skill(false),
            "listening"
        );
        assert_eq!(activity("a08-pairs-th").evidence_skill(false), "listening");
        assert_eq!(
            activity("a14-read-set-class-chat").evidence_skill(false),
            "reading"
        );
        assert_eq!(
            activity("a15-listen-set-putu").evidence_skill(false),
            "listening"
        );
        assert_eq!(
            activity("a16-fix-missing-am").evidence_skill(false),
            "writing"
        );
        assert_eq!(
            activity("a10-speak-introduce").evidence_skill(false),
            "speaking"
        );
        assert_eq!(
            activity("a11-roleplay-classmate").evidence_skill(false),
            "speaking"
        );
        assert_eq!(
            activity("a12-write-introduce").evidence_skill(false),
            "writing"
        );
        assert_eq!(
            activity("a07-read-aloud-thanks").evidence_skill(false),
            "pronunciation"
        );
        assert_eq!(
            activity("a09-shadow-dialogue").evidence_skill(false),
            "pronunciation"
        );
    }

    #[test]
    fn a_mediation_takes_the_skill_of_the_way_it_was_answered() {
        // A mediation is built directly: the example unit has none.
        let common = Common {
            id: "m1".to_owned(),
            skill: Skill::Mediation,
            objective_ids: Vec::new(),
            instructions: Localized {
                en: String::new(),
                id: None,
            },
            scoring: Some(Scoring::Rubric),
        };
        let mediation = Activity::Mediation {
            common,
            source_text: String::new(),
            task: Localized {
                en: String::new(),
                id: None,
            },
            rubric_id: "r".to_owned(),
            model_answers: Vec::new(),
        };
        assert_eq!(mediation.evidence_skill(true), "speaking");
        assert_eq!(mediation.evidence_skill(false), "writing");
    }

    #[test]
    fn a_minimal_pairs_drill_in_say_mode_is_pronunciation_evidence() {
        let mut say = activity("a08-pairs-th");
        if let Activity::MinimalPairs { mode, .. } = &mut say {
            *mode = MinimalPairsMode::SayBoth;
        } else {
            panic!("a08 is minimal pairs");
        }
        assert_eq!(say.evidence_skill(false), "pronunciation");
    }
}
