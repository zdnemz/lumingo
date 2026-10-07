//! The deterministic activity runtime (ASSESSMENT_SPEC section 4): a neutral
//! playable item, the answer a learner gave, and the score `norm/1` produces.
//!
//! Pure logic: nothing here presents an item, plays audio, or touches storage.
//! The caller presents the item (passage, audio, questions), collects the
//! answer, and stores the attempt; this module only decides what an answer is
//! worth. The `curriculum` crate maps its authored activities to [`Item`], so
//! neither crate depends on the other.
//!
//! Only the deterministic types live here: `mcq`, `gap_fill`, `reorder`,
//! `match`, `dictation`, `reading_set`, `listening_set`, `error_correction`,
//! and `minimal_pairs` in listen mode. Productive and pronunciation activities
//! are scored elsewhere (rubric, phoneme engine).

use std::fmt;

use crate::SUCCESS_SCORE;
use crate::deterministic;

/// One question of a reading or listening set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    pub stem: String,
    pub options: Vec<String>,
    /// Index into `options`.
    pub answer_index: usize,
    /// Shown after the answer, in English.
    pub explanation: String,
}

/// Two words a learner must tell apart, or two halves of a match pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pair {
    pub left: String,
    pub right: String,
}

/// One activity to play, in neutral form. The authored activity's `Localized`
/// texts arrive resolved to English by the mapping.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    /// One multiple-choice question. `passage` is shown and `audio_text` is
    /// played before the question, when present.
    Choice {
        stem: String,
        options: Vec<String>,
        answer_index: usize,
        passage: Option<String>,
        audio_text: Option<String>,
        explanation: String,
    },
    GapFill {
        text: String,
        /// Accepted answers per gap, in text order.
        answers: Vec<Vec<String>>,
        explanation: String,
    },
    Reorder {
        tokens: Vec<String>,
        /// The sentence the tokens form.
        answer: String,
        explanation: Option<String>,
    },
    /// Pair each left with its right. The authored order is the correct one;
    /// the caller may shuffle the right column for display and translates the
    /// learner's choices back to authored indexes.
    Match { pairs: Vec<Pair> },
    Dictation {
        audio_text: String,
        accepted: Vec<String>,
    },
    ReadingSet {
        passage: String,
        questions: Vec<Question>,
    },
    /// `replays_allowed` counts extra plays of the audio, not the first one.
    ListeningSet {
        audio_text: String,
        replays_allowed: u32,
        questions: Vec<Question>,
    },
    ErrorCorrection {
        sentence: String,
        accepted: Vec<String>,
        explanation: String,
    },
    /// Listen and choose which of the two words was played (0 = left,
    /// 1 = right).
    MinimalPairs { pairs: Vec<Pair> },
}

/// What the learner answered, in the shape the item asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// One index per question (sets) or per pair (minimal pairs). For one
    /// question the list holds one entry.
    Choices(Vec<usize>),
    /// Typed text: reorder, dictation, error correction.
    Text(String),
    /// One entry per gap, in text order.
    Gaps(Vec<String>),
    /// One chosen right index per left, in the item's pair order.
    Pairs(Vec<usize>),
    /// Minimal pairs, listen mode: per pair, which word was played and which
    /// the learner chose (0 = left, 1 = right). The correct answer is what was
    /// played, so the runtime needs both lists.
    Heard {
        played: Vec<usize>,
        chosen: Vec<usize>,
    },
}

/// What an answer scored.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    /// 0 to 1, the same number an attempt stores as `normalized`.
    pub score: f64,
    /// `score` at or above 0.7 (ASSESSMENT_SPEC section 4).
    pub success: bool,
    /// Gaps that earned half credit for a one-letter slip, for the spelling note.
    pub spelling_notes: Vec<usize>,
    /// Which questions, gaps or pairs were fully correct, for immediate
    /// feedback. One entry for single-answer items.
    pub marks: Vec<bool>,
}

/// The answer does not fit the item: a caller bug, not a learner mistake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnswerMismatch {
    pub expected: &'static str,
    pub got: &'static str,
}

impl fmt::Display for AnswerMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the answer is a {} but the item needs a {}",
            self.got, self.expected
        )
    }
}

impl std::error::Error for AnswerMismatch {}

/// The answer kind an item expects, for mismatch errors.
fn kind(answer: &Answer) -> &'static str {
    match answer {
        Answer::Choices(_) => "choice list",
        Answer::Text(_) => "text",
        Answer::Gaps(_) => "gap list",
        Answer::Pairs(_) => "pair list",
        Answer::Heard { .. } => "heard pair list",
    }
}

fn outcome(score: f64, spelling_notes: Vec<usize>, marks: Vec<bool>) -> Outcome {
    Outcome {
        score,
        success: score >= SUCCESS_SCORE,
        spelling_notes,
        marks,
    }
}

fn mismatch(expected: &'static str, answer: &Answer) -> AnswerMismatch {
    AnswerMismatch {
        expected,
        got: kind(answer),
    }
}

impl Item {
    /// Scores one answer. The answer must be the kind this item asked for; a
    /// mismatched pair is [`AnswerMismatch`], never a silent zero.
    pub fn score(&self, answer: &Answer) -> Result<Outcome, AnswerMismatch> {
        match self {
            Item::Choice { answer_index, .. } => {
                let Answer::Choices(choices) = answer else {
                    return Err(mismatch("choice list", answer));
                };
                let Some(&chosen) = choices.first() else {
                    return Err(mismatch("choice list with one entry", answer));
                };
                let correct = deterministic::score_choice(chosen, *answer_index);
                Ok(outcome(correct, Vec::new(), vec![correct >= 1.0]))
            }
            Item::GapFill { answers, .. } => {
                let Answer::Gaps(given) = answer else {
                    return Err(mismatch("gap list", answer));
                };
                let given: Vec<&str> = given.iter().map(String::as_str).collect();
                let accepted: Vec<Vec<&str>> = answers
                    .iter()
                    .map(|options| options.iter().map(String::as_str).collect())
                    .collect();
                let scored = deterministic::score_gap_fill(&given, &accepted);
                let marks = accepted
                    .iter()
                    .enumerate()
                    .map(|(i, options)| {
                        given.get(i).is_some_and(|g| {
                            options
                                .iter()
                                .any(|o| deterministic::normalize(o) == deterministic::normalize(g))
                        })
                    })
                    .collect();
                Ok(outcome(scored.score, scored.spelling_notes, marks))
            }
            Item::Reorder {
                answer: expected, ..
            } => {
                let Answer::Text(given) = answer else {
                    return Err(mismatch("text", answer));
                };
                let score = deterministic::score_exact(given, &[expected.as_str()]);
                Ok(outcome(score, Vec::new(), vec![score >= 1.0]))
            }
            Item::Match { pairs } => {
                let Answer::Pairs(chosen) = answer else {
                    return Err(mismatch("pair list", answer));
                };
                if chosen.len() != pairs.len() {
                    return Err(mismatch("pair list, one entry per left", answer));
                }
                let marks: Vec<bool> = chosen
                    .iter()
                    .enumerate()
                    .map(|(left, &right)| left == right)
                    .collect();
                let correct = marks.iter().filter(|&&m| m).count();
                Ok(outcome(
                    deterministic::score_share(correct, pairs.len()),
                    Vec::new(),
                    marks,
                ))
            }
            Item::Dictation { accepted, .. } => {
                let Answer::Text(given) = answer else {
                    return Err(mismatch("text", answer));
                };
                let accepted: Vec<&str> = accepted.iter().map(String::as_str).collect();
                let score = deterministic::score_dictation(given, &accepted);
                Ok(outcome(score, Vec::new(), vec![score >= 1.0]))
            }
            Item::ReadingSet { questions, .. } | Item::ListeningSet { questions, .. } => {
                let Answer::Choices(choices) = answer else {
                    return Err(mismatch("choice list", answer));
                };
                if choices.len() != questions.len() {
                    return Err(mismatch("choice list, one entry per question", answer));
                }
                let marks: Vec<bool> = choices
                    .iter()
                    .zip(questions)
                    .map(|(&chosen, question)| chosen == question.answer_index)
                    .collect();
                let correct = marks.iter().filter(|&&m| m).count();
                Ok(outcome(
                    deterministic::score_share(correct, questions.len()),
                    Vec::new(),
                    marks,
                ))
            }
            Item::ErrorCorrection { accepted, .. } => {
                let Answer::Text(given) = answer else {
                    return Err(mismatch("text", answer));
                };
                let accepted: Vec<&str> = accepted.iter().map(String::as_str).collect();
                let score = deterministic::score_exact(given, &accepted);
                Ok(outcome(score, Vec::new(), vec![score >= 1.0]))
            }
            Item::MinimalPairs { pairs } => {
                let Answer::Heard { played, chosen } = answer else {
                    return Err(mismatch("heard pair list", answer));
                };
                if played.len() != pairs.len() || chosen.len() != pairs.len() {
                    return Err(mismatch("heard pair list, one entry per pair", answer));
                }
                let marks: Vec<bool> = played
                    .iter()
                    .zip(chosen)
                    .map(|(&heard, &chosen)| heard == chosen)
                    .collect();
                let correct = marks.iter().filter(|&&m| m).count();
                Ok(outcome(
                    deterministic::score_share(correct, pairs.len()),
                    Vec::new(),
                    marks,
                ))
            }
        }
    }
}

/// One activity being played: the item and its replay budget. The first play of
/// a listening set is free; `replays_allowed` counts the extra plays.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    item: Item,
    replays_left: u32,
}

impl Run {
    /// Starts a run. Listening sets start with their `replays_allowed`; every
    /// other item starts with none.
    pub fn start(item: Item) -> Self {
        let replays_left = match &item {
            Item::ListeningSet {
                replays_allowed, ..
            } => *replays_allowed,
            _ => 0,
        };
        Self { item, replays_left }
    }

    pub fn item(&self) -> &Item {
        &self.item
    }

    /// How many replays the learner may still ask for.
    pub fn replays_left(&self) -> u32 {
        self.replays_left
    }

    /// Consumes one replay. False when none is left, and nothing changes.
    pub fn replay(&mut self) -> bool {
        if self.replays_left == 0 {
            false
        } else {
            self.replays_left -= 1;
            true
        }
    }

    /// Scores one answer against the item, without ending the run.
    pub fn submit(&self, answer: &Answer) -> Result<Outcome, AnswerMismatch> {
        self.item.score(answer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice_item() -> Item {
        Item::Choice {
            stem: "Dewi: \"What's your ___?\"".to_owned(),
            options: vec!["name".to_owned(), "city".to_owned(), "class".to_owned()],
            answer_index: 0,
            passage: None,
            audio_text: Some("Good morning! What's your name?".to_owned()),
            explanation: "She asks about your name.".to_owned(),
        }
    }

    #[test]
    fn mcq_scores_one_or_zero() {
        let item = choice_item();
        let right = item.score(&Answer::Choices(vec![0])).unwrap();
        assert_eq!(right.score, 1.0);
        assert!(right.success);
        assert_eq!(right.marks, [true]);
        let wrong = item.score(&Answer::Choices(vec![2])).unwrap();
        assert_eq!(wrong.score, 0.0);
        assert!(!wrong.success);
    }

    #[test]
    fn gap_fill_expands_contractions_and_notes_one_edit_slips() {
        let item = Item::GapFill {
            text: "Hello! I ___ Arif. I am ___ Makassar.".to_owned(),
            answers: vec![vec!["am".to_owned()], vec!["from".to_owned()]],
            explanation: "Use 'am' after 'I'.".to_owned(),
        };
        // Exact on the first gap; "form" is one edit from "from" but "from"
        // has four letters, so no credit. 1 of 2.
        let scored = item
            .score(&Answer::Gaps(vec!["am".to_owned(), "form".to_owned()]))
            .unwrap();
        assert!((scored.score - 0.5).abs() < 1e-9);
        assert_eq!(scored.marks, [true, false]);
        // A contraction in the table expands: "I'm" fills a gap whose accepted
        // answer is "I am".
        let contracted = Item::GapFill {
            text: "___ happy today.".to_owned(),
            answers: vec![vec!["I am".to_owned()]],
            explanation: String::new(),
        };
        assert_eq!(
            contracted
                .score(&Answer::Gaps(vec!["I'm".to_owned()]))
                .unwrap()
                .score,
            1.0
        );
        // A one-letter slip on a five-letter accepted answer earns half credit
        // and a spelling note.
        let long = Item::GapFill {
            text: "I say ___.".to_owned(),
            answers: vec![vec!["thank".to_owned()]],
            explanation: String::new(),
        };
        let scored = long
            .score(&Answer::Gaps(vec!["thnank".to_owned()]))
            .unwrap();
        assert!((scored.score - 0.5).abs() < 1e-9);
        assert_eq!(scored.spelling_notes, [0]);
        assert_eq!(scored.marks, [false]);
    }

    #[test]
    fn reorder_accepts_a_listed_contraction_and_drops_final_punctuation() {
        let item = Item::Reorder {
            tokens: vec![
                "It".to_owned(),
                "is".to_owned(),
                "my".to_owned(),
                "book".to_owned(),
            ],
            answer: "It is my book".to_owned(),
            explanation: None,
        };
        // "it's" is in the norm/1 table, so it equals "it is".
        assert_eq!(
            item.score(&Answer::Text("it's my book!".to_owned()))
                .unwrap()
                .score,
            1.0
        );
        assert_eq!(
            item.score(&Answer::Text("my book it is".to_owned()))
                .unwrap()
                .score,
            0.0
        );
    }

    #[test]
    fn match_scores_the_share_of_pairs() {
        let item = Item::Match {
            pairs: vec![
                Pair {
                    left: "Good morning".to_owned(),
                    right: "Selamat pagi".to_owned(),
                },
                Pair {
                    left: "Thank you".to_owned(),
                    right: "Terima kasih".to_owned(),
                },
                Pair {
                    left: "See you".to_owned(),
                    right: "Sampai jumpa".to_owned(),
                },
                Pair {
                    left: "Nice to meet you".to_owned(),
                    right: "Senang bertemu dengan Anda".to_owned(),
                },
            ],
        };
        let scored = item.score(&Answer::Pairs(vec![0, 1, 2, 0])).unwrap();
        assert!((scored.score - 0.75).abs() < 1e-9);
        assert!(scored.success);
        assert_eq!(scored.marks, [true, true, true, false]);
    }

    #[test]
    fn dictation_scores_one_minus_the_word_error_rate() {
        let item = Item::Dictation {
            audio_text: "Hello! My name is Dewi. I am from Bandung.".to_owned(),
            accepted: vec!["I am from Bandung".to_owned()],
        };
        assert_eq!(
            item.score(&Answer::Text("I am from Bandung".to_owned()))
                .unwrap()
                .score,
            1.0
        );
        // One substitution against a four-word reference: WER 1/4.
        let scored = item
            .score(&Answer::Text("I am to Bandung".to_owned()))
            .unwrap();
        assert!((scored.score - 0.75).abs() < 1e-9);
        assert!(scored.success);
        // Two missing words against the four-word reference: WER 2/4.
        let scored = item.score(&Answer::Text("I Bandung".to_owned())).unwrap();
        assert!((scored.score - 0.5).abs() < 1e-9);
        assert!(!scored.success);
    }

    #[test]
    fn sets_score_the_share_of_questions() {
        let questions = vec![
            Question {
                stem: "Rudi: \"I am from ___.\"".to_owned(),
                options: vec![
                    "Surabaya".to_owned(),
                    "Medan".to_owned(),
                    "Jakarta".to_owned(),
                ],
                answer_index: 1,
                explanation: "Rudi is from Medan.".to_owned(),
            },
            Question {
                stem: "The teacher:".to_owned(),
                options: vec!["Sari".to_owned(), "Rudi".to_owned(), "Mrs. Lina".to_owned()],
                answer_index: 2,
                explanation: "Mrs. Lina is the teacher.".to_owned(),
            },
        ];
        let item = Item::ReadingSet {
            passage: "Class chat".to_owned(),
            questions,
        };
        let scored = item.score(&Answer::Choices(vec![1, 2])).unwrap();
        assert_eq!(scored.score, 1.0);
        assert_eq!(scored.marks, [true, true]);
        let half = item.score(&Answer::Choices(vec![0, 2])).unwrap();
        assert!((half.score - 0.5).abs() < 1e-9);
        assert_eq!(half.marks, [false, true]);
    }

    #[test]
    fn error_correction_normalises_the_rewrite() {
        let item = Item::ErrorCorrection {
            sentence: "I Dewi.".to_owned(),
            accepted: vec!["I am Dewi.".to_owned(), "I'm Dewi.".to_owned()],
            explanation: "English needs 'am' after 'I'.".to_owned(),
        };
        assert_eq!(
            item.score(&Answer::Text("i am dewi".to_owned()))
                .unwrap()
                .score,
            1.0
        );
        assert_eq!(
            item.score(&Answer::Text("I Dewi.".to_owned()))
                .unwrap()
                .score,
            0.0
        );
    }

    #[test]
    fn minimal_pairs_compare_the_choice_with_what_was_played() {
        let item = Item::MinimalPairs {
            pairs: vec![
                Pair {
                    left: "thank".to_owned(),
                    right: "tank".to_owned(),
                },
                Pair {
                    left: "three".to_owned(),
                    right: "tree".to_owned(),
                },
                Pair {
                    left: "think".to_owned(),
                    right: "sink".to_owned(),
                },
            ],
        };
        let scored = item
            .score(&Answer::Heard {
                played: vec![0, 1, 0],
                chosen: vec![0, 1, 1],
            })
            .unwrap();
        assert!((scored.score - 2.0 / 3.0).abs() < 1e-9);
        assert!(!scored.success);
        assert_eq!(scored.marks, [true, true, false]);
    }

    #[test]
    fn a_success_needs_at_least_point_seven() {
        let item = Item::ReadingSet {
            passage: String::new(),
            questions: vec![
                Question {
                    stem: "a".to_owned(),
                    options: vec!["x".to_owned(), "y".to_owned()],
                    answer_index: 0,
                    explanation: String::new(),
                };
                4
            ],
        };
        // 3 of 4 is a success, 2 of 4 is not.
        let three = item.score(&Answer::Choices(vec![0, 0, 0, 1])).unwrap();
        assert!((three.score - 0.75).abs() < 1e-9);
        assert!(three.success);
        let two = item.score(&Answer::Choices(vec![0, 0, 1, 1])).unwrap();
        assert_eq!(two.score, 0.5);
        assert!(!two.success);
    }

    #[test]
    fn an_answer_of_the_wrong_kind_is_an_error_not_a_zero() {
        let item = choice_item();
        let error = item.score(&Answer::Text("name".to_owned())).unwrap_err();
        assert_eq!(error.expected, "choice list");
        assert_eq!(error.got, "text");
        assert!(error.to_string().contains("choice list"));
        // A set answer with the wrong length is a mismatch too.
        let set = Item::ReadingSet {
            passage: String::new(),
            questions: vec![Question {
                stem: "a".to_owned(),
                options: vec!["x".to_owned()],
                answer_index: 0,
                explanation: String::new(),
            }],
        };
        assert!(set.score(&Answer::Choices(vec![])).is_err());
    }

    #[test]
    fn a_listening_run_spends_its_replay_budget_and_nothing_more() {
        let item = Item::ListeningSet {
            audio_text: "Hello! My name is Putu.".to_owned(),
            replays_allowed: 2,
            questions: vec![Question {
                stem: "Putu: \"I am from ___.\"".to_owned(),
                options: vec!["Bali".to_owned(), "Bandung".to_owned()],
                answer_index: 0,
                explanation: "Putu is from Bali.".to_owned(),
            }],
        };
        let mut run = Run::start(item);
        assert_eq!(run.replays_left(), 2);
        assert!(run.replay());
        assert!(run.replay());
        assert!(!run.replay(), "the budget is spent");
        assert_eq!(run.replays_left(), 0);
        // The run still scores; the budget only gates replays.
        assert_eq!(run.submit(&Answer::Choices(vec![0])).unwrap().score, 1.0);
        // Other items have no replays.
        let mut run = Run::start(choice_item());
        assert_eq!(run.replays_left(), 0);
        assert!(!run.replay());
    }
}
