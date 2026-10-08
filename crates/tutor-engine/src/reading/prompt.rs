//! Prompt assembly for graded reading, contract `reading_passage/1` (T6).

use assessment_engine::Level;

pub const READING_PASSAGE_VERSION: &str = "reading_passage/1";

/// The numbers a generation request is built from: the length range of the
/// level, how many glossary entries and questions are asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadingSpec {
    pub level: Level,
    pub min_words: usize,
    pub max_words: usize,
    pub glossary_count: usize,
    pub question_count: usize,
}

impl ReadingSpec {
    /// The spec of a level. Lengths are the reading table of the curriculum
    /// spec section 4, which graded reading shares with `reading_set`. Questions:
    /// 4 at A1 and A2, 5 at B1 and B2, 6 at C1 and C2. The glossary has 5 to 8
    /// entries; this picks the count by level, a starting value.
    pub fn for_level(level: Level) -> Self {
        let (min_words, max_words) = match level {
            Level::A1 => (60, 100),
            Level::A2 => (100, 180),
            Level::B1 => (180, 300),
            Level::B2 => (300, 450),
            Level::C1 => (450, 650),
            Level::C2 => (600, 800),
        };
        let (glossary_count, question_count) = match level {
            Level::A1 => (5, 4),
            Level::A2 => (6, 4),
            Level::B1 => (6, 5),
            Level::B2 => (7, 5),
            Level::C1 => (8, 6),
            Level::C2 => (8, 6),
        };
        Self {
            level,
            min_words,
            max_words,
            glossary_count,
            question_count,
        }
    }
}

/// The user message. The system prompt carries everything; a structured call
/// still needs a message to answer.
pub const USER_MESSAGE: &str = "Write the text now.";

/// The system prompt, `reading_passage/1`. `topic` is already a clean single
/// line (see `clean_topic`).
pub fn system_prompt(spec: &ReadingSpec, topic: &str, l1_name: &str) -> String {
    let level = spec.level.as_str();
    let ReadingSpec {
        min_words,
        max_words,
        glossary_count,
        question_count,
        ..
    } = spec;
    format!(
        "You write one short reading text for an English learner, with a glossary and comprehension questions. Return only JSON that matches the schema.\n\
         \n\
         - Level: {level}. Topic: {topic}. Length of the passage: {min_words} to {max_words} words.\n\
         - Use vocabulary and grammar that a {level} learner can read. Prefer common words. Use short sentences at A1 and A2.\n\
         - The text is original and suitable for all ages. No real private persons and no brand names. Do not present invented facts as real ones.\n\
         - \"glossary\": the {glossary_count} hardest words or phrases in the passage, each exactly as it appears there, with a gloss in {l1_name} and one new example sentence.\n\
         - \"questions\": exactly {question_count} multiple-choice questions that can be answered from the passage alone. Three or four options each, exactly one correct. \"answer_index\" counts from 0.\n\
         - \"explanation_en\" points to the part of the passage that gives the answer. \"explanation_l1\" says the same in {l1_name}.\n\
         - Plain text only. No markdown, lists, or headings inside the passage."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_golden_prompt_matches_the_stored_file() {
        let spec = ReadingSpec::for_level(Level::A2);
        let prompt = system_prompt(&spec, "A day at the market", "Indonesian");
        let golden = include_str!("../../tests/golden/reading_passage_1.txt");
        assert_eq!(prompt, golden.trim_end_matches('\n'));
    }

    #[test]
    fn the_spec_follows_the_reading_table_and_the_question_counts() {
        let table: Vec<(usize, usize, usize)> = Level::ALL
            .iter()
            .map(|l| {
                let s = ReadingSpec::for_level(*l);
                (s.min_words, s.max_words, s.question_count)
            })
            .collect();
        assert_eq!(
            table,
            [
                (60, 100, 4),
                (100, 180, 4),
                (180, 300, 5),
                (300, 450, 5),
                (450, 650, 6),
                (600, 800, 6)
            ]
        );
        for level in Level::ALL {
            let glossary = ReadingSpec::for_level(level).glossary_count;
            assert!((5..=8).contains(&glossary));
        }
    }

    #[test]
    fn the_ranges_match_the_curriculum_validator_table() {
        for level in Level::ALL {
            let clevel = crate::support::curriculum_level(level);
            let (min, max) = curriculum::validate::reading_range(clevel);
            let spec = ReadingSpec::for_level(level);
            assert_eq!((spec.min_words, spec.max_words), (min, max));
        }
    }
}
