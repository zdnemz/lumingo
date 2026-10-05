//! Extra practice inside a unit (contract T4, S4-10).
//!
//! The model writes items in one flat shape. Each is converted into the typed
//! activity of the unit format, run through the same validators as authored
//! content (`curriculum::validate::validate_generated_item`) and checked against
//! the unit's allowed vocabulary. Invalid items are dropped. Fewer than half
//! valid means one regeneration; after that the learner gets authored items.
//!
//! Generated items are marked generated, stored with their session and never
//! count toward an estimate: attempts on them are written with
//! `counts_toward_estimate = false`, and storage refuses the combination
//! otherwise.

mod prompt;
mod service;

use std::collections::HashSet;
use std::sync::Arc;

use curriculum::validate::text::{lowercase_words, word_set_similarity};
use curriculum::validate::{WordLevels, validate_generated_item};
use curriculum::{Activity, GapFill, GeneratedType, Localized, Mcq, Reorder, Scoring, Skill, Unit};
use serde::{Deserialize, Serialize};

pub use prompt::{
    GrammarRef, PRACTICE_ITEMS_VERSION, PracticeContext, system_prompt as practice_system_prompt,
    user_message as practice_user_message,
};
pub use service::{
    FallbackReason, PracticeAnswer, PracticeConfig, PracticeGenerator, PracticeItem, PracticeSet,
    PracticeSource, authored_fallback, record_practice_attempt,
};

/// The text that identifies an item for the "do not repeat" list.
pub(crate) fn item_text(activity: &Activity) -> Option<String> {
    match activity {
        Activity::Mcq(a) => Some(a.stem.clone()),
        Activity::GapFill(a) => Some(a.text.clone()),
        Activity::Reorder(a) => Some(a.answer.clone()),
        _ => None,
    }
}

/// One generated item as the contract delivers it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawItem {
    #[serde(rename = "type")]
    pub kind: GeneratedType,
    pub objective_id: String,
    pub grammar_id: String,
    pub stem: String,
    pub options: Vec<String>,
    pub answer_index: i64,
    pub text: String,
    pub answers: Vec<Vec<String>>,
    pub tokens: Vec<String>,
    pub answer: String,
    pub explanation_en: String,
    pub explanation_l1: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct RawItems {
    pub items: Vec<RawItem>,
}

/// Why an item was dropped. Kept for reports and tests; it never carries text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rejection {
    TypeNotAllowed,
    UnknownObjective,
    UnknownGrammar,
    /// The item does not have the shape of its type.
    BadShape(&'static str),
    /// A validator of the unit format (E05 to E07) found a problem.
    Validator(String),
    /// The item repeats or lightly rewords an existing one.
    Repeat,
    /// A word is above the unit's maximum level and is not a unit word.
    Vocabulary,
}

/// The words an item may use: the unit's own words, plus any word that a word
/// list puts at or below the unit's generation level.
#[derive(Debug, Clone)]
pub struct Vocabulary {
    unit_words: HashSet<String>,
    levels: Option<Arc<WordLevels>>,
    max_level: curriculum::Level,
}

impl Vocabulary {
    pub fn for_unit(unit: &Unit, levels: Option<Arc<WordLevels>>) -> Self {
        let mut unit_words: HashSet<String> = HashSet::new();
        for (_, text) in
            curriculum::validate::unit_texts(unit, curriculum::validate::TextScope::Everything)
        {
            unit_words.extend(lowercase_words(text));
        }
        for item in &unit.targets.vocabulary {
            unit_words.extend(lowercase_words(&item.lemma));
        }
        Self {
            unit_words,
            levels,
            max_level: unit.generation_policy.max_level,
        }
    }

    /// Without a word list only the unit's own words are known, and a check
    /// against them alone would reject every natural sentence, so it is skipped.
    pub fn is_checked(&self) -> bool {
        self.levels.is_some()
    }

    /// Words of `text` that the word list puts above the level and the unit does
    /// not use. Names (a capital letter inside a sentence) and numbers are exempt.
    pub fn above_level(&self, text: &str) -> Vec<String> {
        let Some(levels) = &self.levels else {
            return Vec::new();
        };
        content_words(text)
            .into_iter()
            .filter(|w| !self.unit_words.contains(w))
            .filter(|w| levels.level_of(w).is_some_and(|l| l > self.max_level))
            .collect()
    }
}

/// Lower-case words of `text` without names and numbers.
pub(crate) fn content_words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut sentence_start = true;
    for piece in text.split_whitespace() {
        let bare = piece.trim_matches(|c: char| !(c.is_alphanumeric() || c == '\''));
        let ends_sentence = piece.ends_with(['.', '?', '!', ':']);
        if !bare.is_empty() {
            let capitalised = bare.chars().next().is_some_and(char::is_uppercase);
            let has_digit = bare.chars().any(|c| c.is_ascii_digit());
            let name_like =
                capitalised && !sentence_start && bare != "I" && !bare.starts_with("I'");
            if !has_digit && !name_like {
                words.extend(lowercase_words(bare));
            }
        }
        sentence_start = ends_sentence;
    }
    words
}

/// Every text of an item that a learner reads.
fn item_texts(raw: &RawItem) -> Vec<String> {
    match raw.kind {
        GeneratedType::Mcq => {
            let mut texts = vec![raw.stem.clone()];
            texts.extend(raw.options.iter().cloned());
            texts
        }
        GeneratedType::GapFill => {
            let mut texts = vec![raw.text.clone()];
            texts.extend(raw.answers.iter().flatten().cloned());
            texts
        }
        GeneratedType::Reorder => vec![raw.answer.clone()],
    }
}

fn folded_set(items: &[String]) -> HashSet<String> {
    items
        .iter()
        .map(|s| {
            s.split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase()
        })
        .collect()
}

/// The shape rules of each type that the schema cannot express.
fn shape_problem(raw: &RawItem) -> Option<&'static str> {
    if raw.explanation_en.trim().is_empty() {
        return Some("no explanation");
    }
    match raw.kind {
        GeneratedType::Mcq => {
            if raw.stem.trim().is_empty() {
                return Some("empty stem");
            }
            if !(3..=4).contains(&raw.options.len()) {
                return Some("an mcq has three or four options");
            }
            if raw.options.iter().any(|o| o.trim().is_empty()) {
                return Some("empty option");
            }
            if folded_set(&raw.options).len() != raw.options.len() {
                return Some("two options are the same");
            }
            let in_range = usize::try_from(raw.answer_index)
                .map(|i| i < raw.options.len())
                .unwrap_or(false);
            if !in_range {
                return Some("answer_index is outside the options");
            }
            None
        }
        GeneratedType::GapFill => {
            let gaps = raw.text.matches("___").count();
            if !(1..=2).contains(&gaps) {
                return Some("a gap_fill has one or two gaps");
            }
            if raw
                .answers
                .iter()
                .any(|list| list.is_empty() || list.iter().any(|a| a.trim().is_empty()))
            {
                return Some("a gap has no accepted answer");
            }
            None
        }
        GeneratedType::Reorder => {
            if raw.tokens.len() < 3 {
                return Some("a reorder item has at least three tokens");
            }
            if raw.answer.trim().is_empty() {
                return Some("empty answer");
            }
            if raw.answer.trim_end().ends_with(['.', '?', '!']) {
                return Some("the answer has final punctuation");
            }
            None
        }
    }
}

fn instructions(kind: GeneratedType, indonesian: bool) -> Localized {
    let (en, id) = match kind {
        GeneratedType::Mcq => ("Choose the correct answer.", "Pilih jawaban yang benar."),
        GeneratedType::GapFill => ("Fill in the gap.", "Isi bagian yang kosong."),
        GeneratedType::Reorder => (
            "Put the words in order.",
            "Susun kata-kata ini menjadi kalimat.",
        ),
    };
    Localized {
        en: en.to_owned(),
        id: indonesian.then(|| id.to_owned()),
    }
}

fn explanation(raw: &RawItem, indonesian: bool) -> Localized {
    Localized {
        en: raw.explanation_en.trim().to_owned(),
        id: (indonesian && !raw.explanation_l1.trim().is_empty())
            .then(|| raw.explanation_l1.trim().to_owned()),
    }
}

/// Converts a raw item into the typed activity. Only the conversion; the
/// checks are in [`check_item`].
pub(crate) fn convert(raw: &RawItem, id: String, indonesian: bool) -> Result<Activity, Rejection> {
    if let Some(problem) = shape_problem(raw) {
        return Err(Rejection::BadShape(problem));
    }
    let common_objectives = vec![raw.objective_id.clone()];
    let activity = match raw.kind {
        GeneratedType::Mcq => {
            let answer_index = u8::try_from(raw.answer_index)
                .map_err(|_| Rejection::BadShape("answer_index is outside the options"))?;
            Activity::Mcq(Mcq {
                id,
                skill: Skill::Grammar,
                objective_ids: common_objectives,
                instructions: instructions(raw.kind, indonesian),
                scoring: Scoring::Deterministic,
                passage: None,
                audio_text: None,
                stem: raw.stem.trim().to_owned(),
                options: raw.options.iter().map(|o| o.trim().to_owned()).collect(),
                answer_index,
                explanation: explanation(raw, indonesian),
            })
        }
        GeneratedType::GapFill => Activity::GapFill(GapFill {
            id,
            skill: Skill::Grammar,
            objective_ids: common_objectives,
            instructions: instructions(raw.kind, indonesian),
            scoring: Scoring::Deterministic,
            text: raw.text.trim().to_owned(),
            answers: raw.answers.clone(),
            explanation: explanation(raw, indonesian),
        }),
        GeneratedType::Reorder => Activity::Reorder(Reorder {
            id,
            skill: Skill::Grammar,
            objective_ids: common_objectives,
            instructions: instructions(raw.kind, indonesian),
            scoring: Scoring::Deterministic,
            tokens: raw.tokens.clone(),
            answer: raw.answer.trim().to_owned(),
            explanation: Some(explanation(raw, indonesian)),
        }),
    };
    Ok(activity)
}

/// Everything an item must satisfy before a learner sees it, and the typed
/// activity when it does. `existing` is the text of the authored items and of the
/// items kept so far, for the repeat check.
pub(crate) fn check_item(
    raw: &RawItem,
    id: String,
    unit: &Unit,
    existing: &[String],
    vocabulary: &Vocabulary,
    indonesian: bool,
) -> Result<Activity, Rejection> {
    let policy = &unit.generation_policy;
    if !policy.allowed_types.contains(&raw.kind) {
        return Err(Rejection::TypeNotAllowed);
    }
    if !unit.objectives.iter().any(|o| o.id == raw.objective_id) {
        return Err(Rejection::UnknownObjective);
    }
    if !policy.allowed_grammar_ids.contains(&raw.grammar_id) {
        return Err(Rejection::UnknownGrammar);
    }
    let activity = convert(raw, id, indonesian)?;
    let findings = validate_generated_item(&activity);
    if let Some(first) = findings.first() {
        return Err(Rejection::Validator(first.code.as_str().to_owned()));
    }
    if let Some(text) = item_text(&activity)
        && existing
            .iter()
            .any(|known| word_set_similarity(known, &text) >= REPEAT_SIMILARITY)
    {
        return Err(Rejection::Repeat);
    }
    if item_texts(raw)
        .iter()
        .any(|t| !vocabulary.above_level(t).is_empty())
    {
        return Err(Rejection::Vocabulary);
    }
    Ok(activity)
}

/// Word-set similarity from which a generated item counts as a repeat. The same
/// threshold as rule W04 of the content validators.
const REPEAT_SIMILARITY: f64 = 0.9;

#[cfg(test)]
mod tests;
