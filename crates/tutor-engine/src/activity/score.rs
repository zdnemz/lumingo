//! Deterministic scoring of the objective activities (assessment spec section 4).
//!
//! The scorers themselves are in `assessment-engine`; this module reads the
//! activity and the response, calls them with the right arguments and shapes
//! the result into a record for the evidence recorder and feedback data for the
//! learner. Confidence is always 1.0 for these, and every answer is compared
//! after the normaliser `norm/1`.

use assessment_engine::{
    score_dictation, score_error_correction, score_gap_fill, score_match, score_mcq, score_reorder,
    score_share,
};
use curriculum::{Activity, Localized, MinimalPairsMode, Question, Scoring};
use serde_json::json;

use super::error::ActivityError;
use super::feedback::{Feedback, ItemFeedback, ItemOutcome};
use super::present::{match_display_order, minimal_pair_spoken};
use super::response::{Response, check_text};
use crate::evidence::DeterministicRecord;

/// A deterministic score with its feedback.
#[derive(Debug, Clone, PartialEq)]
pub struct Scored {
    pub record: DeterministicRecord,
    pub feedback: Feedback,
}

/// The skill an activity's attempt is filed under (assessment spec section 2).
///
/// A listening minimal-pairs item is listening evidence even when its author
/// tagged it with the pronunciation objective, because the learner is judging
/// what they heard. A `mediation` takes the skill of the way it was answered,
/// so it needs to be told: `spoken` is true for a spoken answer.
pub fn evidence_skill(activity: &Activity, spoken: bool) -> &'static str {
    use curriculum::Skill;
    match activity {
        Activity::Mcq(a) => {
            if a.audio_text.is_some() {
                "listening"
            } else if a.passage.is_some() {
                "reading"
            } else {
                match a.skill {
                    Skill::Listening => "listening",
                    Skill::Reading => "reading",
                    Skill::Vocabulary => "vocabulary",
                    _ => "grammar",
                }
            }
        }
        Activity::GapFill(_) | Activity::Reorder(_) => "grammar",
        Activity::Match(_) => "vocabulary",
        Activity::Dictation(_) | Activity::ListeningSet(_) => "listening",
        Activity::MinimalPairs(a) => match a.mode {
            MinimalPairsMode::ListenChoose => "listening",
            MinimalPairsMode::SayBoth => "pronunciation",
        },
        Activity::ReadingSet(_) => "reading",
        Activity::ErrorCorrection(_) | Activity::GuidedWriting(_) => "writing",
        Activity::GuidedSpeaking(_) | Activity::Roleplay(_) => "speaking",
        Activity::Mediation(_) => {
            if spoken {
                "speaking"
            } else {
                "writing"
            }
        }
        Activity::ReadAloud(_) | Activity::Shadowing(_) => "pronunciation",
    }
}

/// What scores an activity: the unit file's `scoring` field, as a name.
pub(crate) fn scoring_name(scoring: Scoring) -> &'static str {
    match scoring {
        Scoring::Deterministic => "a deterministic scorer",
        Scoring::Rubric => "the rubric scorer",
        Scoring::Pron => "the pronunciation engine",
        Scoring::None => "nothing",
    }
}

fn outcome_name(outcome: ItemOutcome) -> &'static str {
    match outcome {
        ItemOutcome::Correct => "correct",
        ItemOutcome::Spelling => "spelling",
        ItemOutcome::Wrong => "wrong",
        ItemOutcome::Blank => "blank",
    }
}

fn expect_picks(response: &Response, expected: usize) -> Result<&[Option<usize>], ActivityError> {
    let Response::Picks(picks) = response else {
        return Err(ActivityError::WrongKind {
            expected: "a list of picks",
        });
    };
    if picks.len() != expected {
        return Err(ActivityError::WrongLength {
            expected,
            got: picks.len(),
        });
    }
    Ok(picks)
}

fn check_pick(pick: Option<usize>, options: usize) -> Result<(), ActivityError> {
    match pick {
        Some(index) if index >= options => Err(ActivityError::OutOfRange { index, options }),
        _ => Ok(()),
    }
}

fn record(
    normalized: f64,
    raw: f64,
    max: f64,
    algorithm: &'static str,
    response_text: String,
    feedback: &Feedback,
    notes: Vec<String>,
) -> DeterministicRecord {
    DeterministicRecord {
        normalized,
        raw,
        max,
        algorithm,
        response_text: Some(response_text),
        details: json!({
            "correct": feedback.correct,
            "total": feedback.total,
            "items": feedback.items.iter().map(|i| outcome_name(i.outcome)).collect::<Vec<_>>(),
        }),
        notes,
    }
}

fn option_text(options: &[String], index: usize) -> String {
    options.get(index).cloned().unwrap_or_default()
}

/// The questions of a reading or listening set.
fn score_questions(
    questions: &[Question],
    response: &Response,
    algorithm: &'static str,
) -> Result<Scored, ActivityError> {
    let picks = expect_picks(response, questions.len())?;
    let mut items = Vec::with_capacity(questions.len());
    let mut right = 0;
    let mut given_text = Vec::new();
    for (index, (question, pick)) in questions.iter().zip(picks).enumerate() {
        check_pick(*pick, question.options.len())?;
        let correct_index = usize::from(question.answer_index);
        let outcome = match pick {
            None => ItemOutcome::Blank,
            Some(chosen) if score_mcq(*chosen, correct_index) == 1.0 => {
                right += 1;
                ItemOutcome::Correct
            }
            Some(_) => ItemOutcome::Wrong,
        };
        let given = pick.map(|i| option_text(&question.options, i));
        given_text.push(given.clone().unwrap_or_else(|| "-".to_owned()));
        items.push(ItemFeedback {
            index,
            outcome,
            given,
            expected: option_text(&question.options, correct_index),
            explanation: Some(question.explanation.clone()),
        });
    }
    let score = score_share(right, questions.len());
    let feedback = Feedback::new(score, items, None);
    Ok(Scored {
        record: record(
            score,
            right as f64,
            questions.len() as f64,
            algorithm,
            given_text.join(" | "),
            &feedback,
            Vec::new(),
        ),
        feedback,
    })
}

fn localized_or_none(text: &Localized) -> Option<Localized> {
    Some(text.clone())
}

/// Scores a response to a deterministic activity.
///
/// Refuses what is not a deterministic activity, a response of the wrong shape or
/// size, and a choice outside the options, before any score exists. A blank
/// answer is not refused: it is a wrong one.
pub fn score_deterministic(
    activity: &Activity,
    response: &Response,
) -> Result<Scored, ActivityError> {
    match activity {
        Activity::Mcq(a) => {
            let Response::Choice(chosen) = response else {
                return Err(ActivityError::WrongKind {
                    expected: "a choice",
                });
            };
            if *chosen >= a.options.len() {
                return Err(ActivityError::OutOfRange {
                    index: *chosen,
                    options: a.options.len(),
                });
            }
            let correct_index = usize::from(a.answer_index);
            let score = score_mcq(*chosen, correct_index);
            let outcome = if score == 1.0 {
                ItemOutcome::Correct
            } else {
                ItemOutcome::Wrong
            };
            let given = option_text(&a.options, *chosen);
            let feedback = Feedback::new(
                score,
                vec![ItemFeedback {
                    index: 0,
                    outcome,
                    given: Some(given.clone()),
                    expected: option_text(&a.options, correct_index),
                    explanation: None,
                }],
                localized_or_none(&a.explanation),
            );
            Ok(Scored {
                record: record(score, score, 1.0, "mcq/1", given, &feedback, Vec::new()),
                feedback,
            })
        }
        Activity::GapFill(a) => {
            let Response::Gaps(answers) = response else {
                return Err(ActivityError::WrongKind {
                    expected: "gap answers",
                });
            };
            if answers.len() != a.answers.len() {
                return Err(ActivityError::WrongLength {
                    expected: a.answers.len(),
                    got: answers.len(),
                });
            }
            for answer in answers {
                check_text(answer)?;
            }
            let whole = score_gap_fill(answers, &a.answers);
            let mut items = Vec::with_capacity(answers.len());
            let mut notes = Vec::new();
            for (index, (given, accepted)) in answers.iter().zip(&a.answers).enumerate() {
                let single =
                    score_gap_fill(std::slice::from_ref(given), std::slice::from_ref(accepted));
                let outcome = if given.trim().is_empty() {
                    ItemOutcome::Blank
                } else if single.score >= 1.0 {
                    ItemOutcome::Correct
                } else if single.score > 0.0 {
                    notes.push(format!(
                        "gap {}: one letter off \"{}\"",
                        index + 1,
                        accepted.first().map_or("", String::as_str)
                    ));
                    ItemOutcome::Spelling
                } else {
                    ItemOutcome::Wrong
                };
                items.push(ItemFeedback {
                    index,
                    outcome,
                    given: Some(given.clone()),
                    expected: accepted.first().cloned().unwrap_or_default(),
                    explanation: None,
                });
            }
            let feedback = Feedback::new(whole.score, items, localized_or_none(&a.explanation));
            let correct_gaps = whole.score * a.answers.len() as f64;
            Ok(Scored {
                record: record(
                    whole.score,
                    correct_gaps,
                    a.answers.len() as f64,
                    "gap_fill/1",
                    answers.join(" | "),
                    &feedback,
                    notes,
                ),
                feedback,
            })
        }
        Activity::Reorder(a) => {
            let Response::Order(given) = response else {
                return Err(ActivityError::WrongKind {
                    expected: "an order of tokens",
                });
            };
            if given.len() != a.tokens.len() {
                return Err(ActivityError::WrongLength {
                    expected: a.tokens.len(),
                    got: given.len(),
                });
            }
            for token in given {
                check_text(token)?;
            }
            let expected: Vec<String> = a.answer.split_whitespace().map(str::to_owned).collect();
            let score = score_reorder(given, &expected);
            let joined = given.join(" ");
            let feedback = Feedback::new(
                score,
                vec![ItemFeedback {
                    index: 0,
                    outcome: if score == 1.0 {
                        ItemOutcome::Correct
                    } else {
                        ItemOutcome::Wrong
                    },
                    given: Some(joined.clone()),
                    expected: a.answer.clone(),
                    explanation: None,
                }],
                a.explanation.clone(),
            );
            Ok(Scored {
                record: record(
                    score,
                    score,
                    1.0,
                    "reorder/1",
                    joined,
                    &feedback,
                    Vec::new(),
                ),
                feedback,
            })
        }
        Activity::Match(a) => {
            let picks = expect_picks(response, a.pairs.len())?;
            let order = match_display_order(&a.id, a.pairs.len());
            let mut original: Vec<Option<usize>> = Vec::with_capacity(picks.len());
            for pick in picks {
                check_pick(*pick, a.pairs.len())?;
                original.push(pick.and_then(|shown| order.get(shown).copied()));
            }
            let correct: Vec<usize> = (0..a.pairs.len()).collect();
            let score = score_match(&original, &correct);
            let mut items = Vec::with_capacity(a.pairs.len());
            let mut given_text = Vec::new();
            for (index, pair) in a.pairs.iter().enumerate() {
                let (outcome, given) = match original[index] {
                    None => (ItemOutcome::Blank, None),
                    Some(chosen) => (
                        if chosen == index {
                            ItemOutcome::Correct
                        } else {
                            ItemOutcome::Wrong
                        },
                        a.pairs.get(chosen).map(|p| p.right.clone()),
                    ),
                };
                given_text.push(format!(
                    "{} = {}",
                    pair.left,
                    given.clone().unwrap_or_else(|| "-".to_owned())
                ));
                items.push(ItemFeedback {
                    index,
                    outcome,
                    given,
                    expected: pair.right.clone(),
                    explanation: None,
                });
            }
            let feedback = Feedback::new(score, items, None);
            let right = score * a.pairs.len() as f64;
            Ok(Scored {
                record: record(
                    score,
                    right,
                    a.pairs.len() as f64,
                    "match/1",
                    given_text.join(" | "),
                    &feedback,
                    Vec::new(),
                ),
                feedback,
            })
        }
        Activity::Dictation(a) => {
            let Response::Text(text) = response else {
                return Err(ActivityError::WrongKind { expected: "text" });
            };
            check_text(text)?;
            let score = score_dictation(text, &a.accepted_answers);
            // The accepted answer the learner was closest to, to show.
            let best = a
                .accepted_answers
                .iter()
                .max_by(|x, y| {
                    score_dictation(text, std::slice::from_ref(*x))
                        .total_cmp(&score_dictation(text, std::slice::from_ref(*y)))
                })
                .cloned()
                .unwrap_or_default();
            let outcome = if text.trim().is_empty() {
                ItemOutcome::Blank
            } else if score >= 1.0 {
                ItemOutcome::Correct
            } else {
                ItemOutcome::Wrong
            };
            let feedback = Feedback::new(
                score,
                vec![ItemFeedback {
                    index: 0,
                    outcome,
                    given: Some(text.clone()),
                    expected: best,
                    explanation: None,
                }],
                None,
            );
            Ok(Scored {
                record: record(
                    score,
                    score,
                    1.0,
                    "dictation/1",
                    text.clone(),
                    &feedback,
                    Vec::new(),
                ),
                feedback,
            })
        }
        Activity::MinimalPairs(a) if a.mode == MinimalPairsMode::ListenChoose => {
            let picks = expect_picks(response, a.pairs.len())?;
            let mut items = Vec::with_capacity(a.pairs.len());
            let mut right = 0;
            let mut given_text = Vec::new();
            for (index, (pair, pick)) in a.pairs.iter().zip(picks).enumerate() {
                check_pick(*pick, 2)?;
                let spoken = minimal_pair_spoken(&a.id, index);
                let word = |i: usize| {
                    if i == 0 {
                        pair.a.clone()
                    } else {
                        pair.b.clone()
                    }
                };
                let outcome = match pick {
                    None => ItemOutcome::Blank,
                    Some(chosen) if *chosen == spoken => {
                        right += 1;
                        ItemOutcome::Correct
                    }
                    Some(_) => ItemOutcome::Wrong,
                };
                let given = pick.map(word);
                given_text.push(given.clone().unwrap_or_else(|| "-".to_owned()));
                items.push(ItemFeedback {
                    index,
                    outcome,
                    given,
                    expected: word(spoken),
                    explanation: None,
                });
            }
            let score = score_share(right, a.pairs.len());
            let feedback = Feedback::new(score, items, None);
            Ok(Scored {
                record: record(
                    score,
                    right as f64,
                    a.pairs.len() as f64,
                    "minimal_pairs_listen/1",
                    given_text.join(" | "),
                    &feedback,
                    Vec::new(),
                ),
                feedback,
            })
        }
        Activity::ReadingSet(a) => score_questions(&a.questions, response, "reading_set/1"),
        Activity::ListeningSet(a) => score_questions(&a.questions, response, "listening_set/1"),
        Activity::ErrorCorrection(a) => {
            let Response::Text(text) = response else {
                return Err(ActivityError::WrongKind { expected: "text" });
            };
            check_text(text)?;
            let score = score_error_correction(text, &a.accepted_answers);
            let outcome = if text.trim().is_empty() {
                ItemOutcome::Blank
            } else if score == 1.0 {
                ItemOutcome::Correct
            } else {
                ItemOutcome::Wrong
            };
            let feedback = Feedback::new(
                score,
                vec![ItemFeedback {
                    index: 0,
                    outcome,
                    given: Some(text.clone()),
                    expected: a.accepted_answers.first().cloned().unwrap_or_default(),
                    explanation: None,
                }],
                localized_or_none(&a.explanation),
            );
            Ok(Scored {
                record: record(
                    score,
                    score,
                    1.0,
                    "error_correction/1",
                    text.clone(),
                    &feedback,
                    Vec::new(),
                ),
                feedback,
            })
        }
        other => Err(ActivityError::NotDeterministic(scoring_name(
            other.common().scoring,
        ))),
    }
}
