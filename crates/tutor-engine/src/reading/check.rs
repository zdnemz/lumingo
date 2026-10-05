//! The local checks of T6. Any failure makes the text unusable.

use std::collections::HashSet;

use curriculum::validate::WordLevels;
use curriculum::validate::text::{lowercase_words, word_count};

use crate::practice::content_words;
use crate::support::{curriculum_level, fold};

use super::RawReading;
use super::prompt::ReadingSpec;

/// The passage may be this much shorter or longer than the range of its level.
const LENGTH_TOLERANCE: f64 = 0.10;
/// At most this share of running words may be above the level.
const MAX_ABOVE_LEVEL_SHARE: f64 = 0.08;

/// Why a generated text is not usable. It names the problem and carries no text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadingProblem {
    Empty(&'static str),
    TooShort {
        words: usize,
    },
    TooLong {
        words: usize,
    },
    /// More than 8 percent of the running words are above the level.
    TooHard,
    /// The passage has markdown, a list or a heading.
    Markup,
    /// The glossary entry at this position does not occur in the passage.
    GlossaryNotInPassage(usize),
    /// The question at this position does not have three or four different options.
    OptionCount(usize),
    /// The question at this position has an answer_index outside its options.
    AnswerIndex(usize),
    /// The question at this position has the same stem as an earlier one.
    DuplicateStem(usize),
}

fn has_markup(passage: &str) -> bool {
    if passage.contains("**") || passage.contains("```") {
        return true;
    }
    passage.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with('#')
            || line.starts_with("* ")
            || line.starts_with("- ")
            || line
                .split_once(". ")
                .is_some_and(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
    })
}

/// Runs the checks. `levels` is the word list for the vocabulary profile; without
/// one that check cannot run, and the caller reports it as not done.
pub fn check_reading(
    reading: &RawReading,
    spec: &ReadingSpec,
    levels: Option<&WordLevels>,
) -> Vec<ReadingProblem> {
    let mut problems = Vec::new();
    if reading.passage.trim().is_empty() {
        return vec![ReadingProblem::Empty("passage")];
    }
    if reading.glossary.is_empty() {
        problems.push(ReadingProblem::Empty("glossary"));
    }
    if reading.questions.is_empty() {
        problems.push(ReadingProblem::Empty("questions"));
    }

    let words = word_count(&reading.passage);
    if (words as f64) < spec.min_words as f64 * (1.0 - LENGTH_TOLERANCE) {
        problems.push(ReadingProblem::TooShort { words });
    }
    if (words as f64) > spec.max_words as f64 * (1.0 + LENGTH_TOLERANCE) {
        problems.push(ReadingProblem::TooLong { words });
    }
    if has_markup(&reading.passage) {
        problems.push(ReadingProblem::Markup);
    }

    let passage = fold(&reading.passage);
    for (i, entry) in reading.glossary.iter().enumerate() {
        let word = fold(&entry.word);
        if word.is_empty() || !passage.contains(&word) {
            problems.push(ReadingProblem::GlossaryNotInPassage(i));
        }
    }

    if let Some(levels) = levels {
        let glossary_words: HashSet<String> = reading
            .glossary
            .iter()
            .flat_map(|g| lowercase_words(&g.word))
            .collect();
        let running: Vec<String> = content_words(&reading.passage)
            .into_iter()
            .filter(|w| !glossary_words.contains(w))
            .collect();
        let above = running
            .iter()
            .filter(|w| {
                levels
                    .level_of(w)
                    .is_some_and(|l| l > curriculum_level(spec.level))
            })
            .count();
        if !running.is_empty() && above as f64 / running.len() as f64 > MAX_ABOVE_LEVEL_SHARE {
            problems.push(ReadingProblem::TooHard);
        }
    }

    let mut stems: HashSet<String> = HashSet::new();
    for (i, question) in reading.questions.iter().enumerate() {
        let options: HashSet<String> = question.options.iter().map(|o| fold(o)).collect();
        let distinct_non_empty = options.len() == question.options.len() && !options.contains("");
        if !(3..=4).contains(&question.options.len()) || !distinct_non_empty {
            problems.push(ReadingProblem::OptionCount(i));
        }
        let in_range = usize::try_from(question.answer_index)
            .is_ok_and(|index| index < question.options.len());
        if !in_range {
            problems.push(ReadingProblem::AnswerIndex(i));
        }
        let stem = fold(&question.stem);
        if stem.is_empty() || !stems.insert(stem) {
            problems.push(ReadingProblem::DuplicateStem(i));
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use assessment_engine::Level;

    use crate::reading::{GlossaryEntry, ReadingQuestion};

    /// A passage of exactly `n` plain words.
    fn passage_of(n: usize) -> String {
        let mut words: Vec<String> = Vec::new();
        for i in 0..n {
            words.push(["market", "sells", "fresh", "fruit", "every", "morning"][i % 6].to_owned());
        }
        let mut text = words.join(" ");
        text.push('.');
        text
    }

    fn question(stem: &str) -> ReadingQuestion {
        ReadingQuestion {
            stem: stem.into(),
            options: vec!["fruit".into(), "shoes".into(), "books".into()],
            answer_index: 0,
            explanation_en: "See the first line.".into(),
            explanation_l1: "Lihat baris pertama.".into(),
        }
    }

    fn reading(words: usize) -> RawReading {
        RawReading {
            title: "The market".into(),
            passage: passage_of(words),
            glossary: vec![GlossaryEntry {
                word: "fresh fruit".into(),
                gloss_l1: "buah segar".into(),
                example: "I like fresh fruit.".into(),
            }],
            questions: vec![
                question("What does it sell?"),
                question("When does it sell?"),
            ],
        }
    }

    fn spec() -> ReadingSpec {
        ReadingSpec::for_level(Level::A1)
    }

    #[test]
    fn a_text_inside_the_range_with_valid_parts_passes() {
        assert_eq!(check_reading(&reading(80), &spec(), None), []);
    }

    #[test]
    fn length_has_a_ten_percent_tolerance_at_both_ends() {
        let cases = [
            (53, false),
            (54, true),
            (60, true),
            (100, true),
            (110, true),
            (111, false),
        ];
        for (words, ok) in cases {
            let problems = check_reading(&reading(words), &spec(), None);
            assert_eq!(problems.is_empty(), ok, "{words} words: {problems:?}");
        }
        assert!(matches!(
            check_reading(&reading(40), &spec(), None)[..],
            [ReadingProblem::TooShort { words: 40 }]
        ));
        assert!(matches!(
            check_reading(&reading(150), &spec(), None)[..],
            [ReadingProblem::TooLong { words: 150 }]
        ));
    }

    #[test]
    fn a_glossary_word_must_occur_in_the_passage() {
        let mut r = reading(80);
        r.glossary.push(GlossaryEntry {
            word: "elephant".into(),
            gloss_l1: "gajah".into(),
            example: "An elephant is big.".into(),
        });
        assert_eq!(
            check_reading(&r, &spec(), None),
            [ReadingProblem::GlossaryNotInPassage(1)]
        );
        r.glossary[1].word = "FRESH   Fruit".into();
        assert_eq!(check_reading(&r, &spec(), None), []);
    }

    #[test]
    fn questions_need_three_or_four_different_options_and_an_answer_inside_them() {
        let mut r = reading(80);
        r.questions[0].options = vec!["fruit".into(), "shoes".into()];
        r.questions[1].answer_index = 3;
        assert_eq!(
            check_reading(&r, &spec(), None),
            [
                ReadingProblem::OptionCount(0),
                ReadingProblem::AnswerIndex(1)
            ]
        );
        r.questions[0].options = vec!["fruit".into(), "Fruit".into(), "shoes".into()];
        r.questions[1].answer_index = -1;
        assert_eq!(
            check_reading(&r, &spec(), None),
            [
                ReadingProblem::OptionCount(0),
                ReadingProblem::AnswerIndex(1)
            ]
        );
        r.questions[0].options = ["a", "b", "c", "d", "e"].map(String::from).to_vec();
        r.questions[1].answer_index = 2;
        assert_eq!(
            check_reading(&r, &spec(), None),
            [ReadingProblem::OptionCount(0)]
        );
    }

    #[test]
    fn two_questions_with_the_same_stem_are_invalid() {
        let mut r = reading(80);
        r.questions[1].stem = "  WHAT does it   sell? ".into();
        assert_eq!(
            check_reading(&r, &spec(), None),
            [ReadingProblem::DuplicateStem(1)]
        );
    }

    #[test]
    fn markup_in_the_passage_is_invalid() {
        for markup in [
            "# Title\nplain",
            "plain **bold**",
            "- one\n- two",
            "1. one\n2. two",
        ] {
            let mut r = reading(80);
            r.passage = format!("{} {}", markup, passage_of(80));
            assert!(
                check_reading(&r, &spec(), None).contains(&ReadingProblem::Markup),
                "{markup}"
            );
        }
        let mut r = reading(80);
        r.passage = format!("Dewi is 3.5 years old. {}", passage_of(75));
        assert!(!check_reading(&r, &spec(), None).contains(&ReadingProblem::Markup));
    }

    #[test]
    fn empty_parts_are_invalid() {
        let mut r = reading(80);
        r.glossary.clear();
        r.questions.clear();
        assert_eq!(
            check_reading(&r, &spec(), None),
            [
                ReadingProblem::Empty("glossary"),
                ReadingProblem::Empty("questions")
            ]
        );
        r.passage = "  ".into();
        assert_eq!(
            check_reading(&r, &spec(), None),
            [ReadingProblem::Empty("passage")]
        );
    }

    #[test]
    fn the_vocabulary_profile_allows_up_to_eight_percent_above_level() {
        let levels = WordLevels::parse("market,A1\nsells,A1\nbureaucracy,C1\n").expect("list");
        // 100 words: `hard` of them C1, one "sells" (the glossary word, not counted)
        // and the rest "market". So the profile is judged on 99 running words.
        let build = |hard: usize| {
            let mut words: Vec<&str> = vec!["bureaucracy"; hard];
            words.push("sells");
            words.extend(std::iter::repeat_n("market", 99 - hard));
            let mut r = reading(80);
            r.passage = format!("{}.", words.join(" "));
            r.glossary = vec![GlossaryEntry {
                word: "sells".into(),
                gloss_l1: "menjual".into(),
                example: "She sells fruit.".into(),
            }];
            r
        };
        let problems = |hard: usize| check_reading(&build(hard), &spec(), Some(&levels));
        assert_eq!(problems(7), [], "7 of 99 is 7.1 percent");
        assert_eq!(
            problems(8),
            [ReadingProblem::TooHard],
            "8 of 99 is 8.1 percent"
        );
    }

    #[test]
    fn names_and_unknown_words_do_not_count_as_above_level() {
        let levels = WordLevels::parse("market,A1\nzoo,C2\n").expect("list");
        let mut r = reading(80);
        r.passage = format!(
            "Today Dewi and Zoo Wang go to the market. {}",
            passage_of(70)
        );
        r.glossary[0].word = "fresh fruit".into();
        assert!(!check_reading(&r, &spec(), Some(&levels)).contains(&ReadingProblem::TooHard));
    }
}
