//! Prompt assembly for extra practice, contract `practice_items/1` (T4).

use assessment_engine::Level;
use curriculum::{GeneratedType, Unit};
use serde::Serialize;

use crate::analysis::ObjectiveRef;

pub const PRACTICE_ITEMS_VERSION: &str = "practice_items/1";

pub(crate) fn type_name(kind: GeneratedType) -> &'static str {
    match kind {
        GeneratedType::Mcq => "mcq",
        GeneratedType::GapFill => "gap_fill",
        GeneratedType::Reorder => "reorder",
    }
}

/// The system prompt, `practice_items/1`.
pub fn system_prompt(
    count: usize,
    allowed: &[GeneratedType],
    max_level: Level,
    l1_name: &str,
) -> String {
    let allowed_types = allowed
        .iter()
        .map(|t| type_name(*t))
        .collect::<Vec<_>>()
        .join(", ");
    let max_level = max_level.as_str();
    format!(
        "You write extra practice items for one unit of an English course. Return only JSON that matches the schema.\n\
         \n\
         - Write exactly {count} items. Use only these item types: {allowed_types}.\n\
         - Every item practises one of the grammar points given and names its \"grammar_id\" and one \"objective_id\" from the input.\n\
         - Use only words from the vocabulary list in the input, plus names of people and places.\n\
         - Sentences must be natural, correct English at level {max_level} or below.\n\
         - Do not repeat or lightly reword the existing items given in the input.\n\
         - mcq: fill \"stem\", three or four \"options\", and \"answer_index\". Exactly one option is correct. Leave \"text\", \"answers\", \"tokens\", \"answer\" empty.\n\
         - gap_fill: fill \"text\" with one or two gaps written as three underscores, and \"answers\" with one list of accepted answers per gap. Leave \"stem\", \"options\", \"tokens\", \"answer\" empty and set \"answer_index\" to -1.\n\
         - reorder: fill \"tokens\" with the words in mixed order and \"answer\" with the correct sentence without final punctuation. Leave the other item fields empty and set \"answer_index\" to -1.\n\
         - \"explanation_en\" is one short sentence. \"explanation_l1\" is the same in {l1_name}."
    )
}

/// A grammar point the items may practise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GrammarRef {
    pub id: String,
    pub name: String,
    pub pattern: String,
}

/// The user message of the generation call. Field order is the wire order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PracticeContext {
    pub unit_title: String,
    pub max_level: String,
    pub grammar: Vec<GrammarRef>,
    pub objectives: Vec<ObjectiveRef>,
    pub vocabulary: Vec<String>,
    /// The stem, text or answer of every authored item of an allowed type, and of
    /// items generated earlier in the session, so the model does not repeat them.
    pub existing_items: Vec<String>,
}

impl PracticeContext {
    /// The context of a unit: the grammar points its policy allows, every
    /// objective, the vocabulary targets and the items already there.
    pub fn from_unit(unit: &Unit, earlier_generated: &[String]) -> Self {
        let policy = &unit.generation_policy;
        let grammar = unit
            .targets
            .grammar
            .iter()
            .filter(|g| policy.allowed_grammar_ids.contains(&g.id))
            .map(|g| GrammarRef {
                id: g.id.clone(),
                name: g.name.clone(),
                pattern: g.pattern.clone(),
            })
            .collect();
        let objectives = unit
            .objectives
            .iter()
            .map(|o| ObjectiveRef {
                id: o.id.clone(),
                can_do: o.can_do.en.clone(),
            })
            .collect();
        let vocabulary = unit
            .targets
            .vocabulary
            .iter()
            .map(|v| v.lemma.clone())
            .collect();
        let mut existing_items: Vec<String> = unit
            .activities
            .iter()
            .filter_map(crate::practice::item_text)
            .collect();
        existing_items.extend(earlier_generated.iter().cloned());
        Self {
            unit_title: unit.title.en.clone(),
            max_level: policy.max_level.as_str().to_owned(),
            grammar,
            objectives,
            vocabulary,
            existing_items,
        }
    }
}

/// The user message: one JSON object.
pub fn user_message(context: &PracticeContext) -> String {
    serde_json::to_string(context).unwrap_or_else(|_| "{}".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_golden_prompt_matches_the_stored_file() {
        let prompt = system_prompt(
            4,
            &[
                GeneratedType::Mcq,
                GeneratedType::GapFill,
                GeneratedType::Reorder,
            ],
            Level::A1,
            "Indonesian",
        );
        let golden = include_str!("../../tests/golden/practice_items_1.txt");
        assert_eq!(prompt, golden.trim_end_matches('\n'));
    }

    #[test]
    fn only_the_allowed_types_are_named() {
        let prompt = system_prompt(2, &[GeneratedType::Mcq], Level::B1, "Indonesian");
        assert!(prompt.contains("Use only these item types: mcq."));
        assert!(prompt.contains("English at level B1 or below"));
    }

    #[test]
    fn the_golden_user_message_matches_the_stored_file() {
        let context = PracticeContext {
            unit_title: "Hello! Nice to meet you".into(),
            max_level: "A1".into(),
            grammar: vec![GrammarRef {
                id: "g-be-i-am".into(),
                name: "I am".into(),
                pattern: "I am + noun".into(),
            }],
            objectives: vec![ObjectiveRef {
                id: "o2-introduce".into(),
                can_do: "I can say who I am.".into(),
            }],
            vocabulary: vec!["name".into(), "student".into()],
            existing_items: vec!["I ___ a student.".into()],
        };
        let golden = include_str!("../../tests/golden/practice_items_1_user.json");
        assert_eq!(user_message(&context), golden.trim_end_matches('\n'));
    }
}
