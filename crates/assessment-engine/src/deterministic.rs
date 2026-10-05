//! Scores for objective items (assessment spec, section 4).
//!
//! These functions take what the learner answered and what is accepted, and
//! return a score from 0 to 1. Confidence is always 1.0 for these, and an
//! attempt counts as a success when its score is at least [`SUCCESS_THRESHOLD`].

use crate::normalise::{edit_distance, normalize};

/// A score at or above this is a success.
pub const SUCCESS_THRESHOLD: f64 = 0.7;

pub fn is_success(score: f64) -> bool {
    score >= SUCCESS_THRESHOLD
}

/// A gap answer this close to an accepted answer of at least this many letters
/// earns half credit and a spelling note.
const NEAR_MISS_MIN_LETTERS: usize = 5;

/// 1 for the right option, otherwise 0.
pub fn score_mcq(chosen: usize, correct: usize) -> f64 {
    if chosen == correct { 1.0 } else { 0.0 }
}

/// Share of `total` that is `correct`. Used for question sets and minimal pairs.
/// An empty set scores 0 rather than dividing by zero.
pub fn score_share(correct: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        correct as f64 / total as f64
    }
}

/// The result of a gap-fill item.
#[derive(Debug, Clone, PartialEq)]
pub struct GapScore {
    pub score: f64,
    /// Indexes of gaps that earned half credit because of a small spelling slip.
    pub spelling_notes: Vec<usize>,
}

/// Share of gaps correct. `answers[i]` is checked against `accepted[i]`, the
/// list of accepted answers for that gap. A missing answer is wrong. A wrong
/// answer one edit away from an accepted answer of five letters or more earns 0.5.
pub fn score_gap_fill(answers: &[String], accepted: &[Vec<String>]) -> GapScore {
    if accepted.is_empty() {
        return GapScore {
            score: 0.0,
            spelling_notes: Vec::new(),
        };
    }
    let mut total = 0.0;
    let mut notes = Vec::new();
    for (index, options) in accepted.iter().enumerate() {
        let given = answers.get(index).map(|a| normalize(a)).unwrap_or_default();
        if given.is_empty() {
            continue;
        }
        let normalised: Vec<String> = options.iter().map(|o| normalize(o)).collect();
        if normalised.contains(&given) {
            total += 1.0;
        } else if normalised.iter().any(|option| {
            option.chars().count() >= NEAR_MISS_MIN_LETTERS && edit_distance(option, &given) == 1
        }) {
            total += 0.5;
            notes.push(index);
        }
    }
    GapScore {
        score: total / accepted.len() as f64,
        spelling_notes: notes,
    }
}

/// 1 when the tokens are in the accepted order, otherwise 0.
pub fn score_reorder(given: &[String], answer: &[String]) -> f64 {
    let same = given.len() == answer.len()
        && given
            .iter()
            .zip(answer)
            .all(|(a, b)| normalize(a) == normalize(b));
    if same { 1.0 } else { 0.0 }
}

/// Share of pairs matched correctly. `given[i]` is the right-hand index the
/// learner chose for left-hand item `i`, and `correct[i]` the right one.
pub fn score_match(given: &[Option<usize>], correct: &[usize]) -> f64 {
    let right = correct
        .iter()
        .enumerate()
        .filter(|(index, expected)| given.get(*index).copied().flatten() == Some(**expected))
        .count();
    score_share(right, correct.len())
}

/// The words of a text, for comparing what was heard with what was said.
/// Punctuation touching a word is not part of the word: a dictation is not
/// marked down for a missing comma.
fn words_of(text: &str) -> Vec<String> {
    normalize(text)
        .split_whitespace()
        .map(|token| {
            token
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_owned()
        })
        .filter(|word| !word.is_empty())
        .collect()
}

/// Word-level edit distance between two word lists.
fn word_distance(a: &[String], b: &[String]) -> usize {
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, wa) in a.iter().enumerate() {
        let mut current = vec![i + 1];
        for (j, wb) in b.iter().enumerate() {
            let cost = usize::from(wa != wb);
            let value = (previous[j] + cost)
                .min(previous[j + 1] + 1)
                .min(current[j] + 1);
            current.push(value);
        }
        previous = current;
    }
    previous[b.len()]
}

/// 1 minus the word error rate against the best accepted answer, never below 0.
pub fn score_dictation(response: &str, accepted: &[String]) -> f64 {
    let heard = words_of(response);
    accepted
        .iter()
        .map(|reference| {
            let reference = words_of(reference);
            if reference.is_empty() {
                return 0.0;
            }
            let rate = word_distance(&reference, &heard) as f64 / reference.len() as f64;
            (1.0 - rate).max(0.0)
        })
        .fold(0.0, f64::max)
}

/// 1 when the rewritten sentence equals an accepted answer after normalisation, otherwise 0.
pub fn score_error_correction(response: &str, accepted: &[String]) -> f64 {
    let given = normalize(response);
    if !given.is_empty() && accepted.iter().any(|a| normalize(a) == given) {
        1.0
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|i| (*i).to_owned()).collect()
    }

    #[test]
    fn mcq_is_one_or_zero() {
        assert_eq!(score_mcq(2, 2), 1.0);
        assert_eq!(score_mcq(0, 2), 0.0);
    }

    #[test]
    fn a_set_scores_the_share_correct_and_an_empty_set_scores_zero() {
        assert_eq!(score_share(3, 4), 0.75);
        assert_eq!(score_share(0, 0), 0.0);
    }

    #[test]
    fn the_success_line_is_seven_tenths() {
        assert!(is_success(0.7));
        assert!(!is_success(0.699));
    }

    #[test]
    fn gap_fill_scores_the_share_of_gaps() {
        let accepted = vec![s(&["am", "'m"]), s(&["is"])];
        let result = score_gap_fill(&s(&["am", "are"]), &accepted);
        assert_eq!(result.score, 0.5);
        assert!(result.spelling_notes.is_empty());
        assert_eq!(score_gap_fill(&s(&["AM", "Is."]), &accepted).score, 1.0);
    }

    #[test]
    fn gap_fill_gives_half_credit_for_one_slip_in_a_long_word() {
        let accepted = vec![s(&["beautiful"])];
        let result = score_gap_fill(&s(&["beautifull"]), &accepted);
        assert_eq!(result.score, 0.5);
        assert_eq!(result.spelling_notes, [0]);
        // Two neighbouring letters swapped is one edit, so it still earns half credit.
        assert_eq!(score_gap_fill(&s(&["beatuiful"]), &accepted).score, 0.5);
        // Two separate slips are more than one edit.
        assert_eq!(score_gap_fill(&s(&["beutifull"]), &accepted).score, 0.0);
    }

    #[test]
    fn a_short_word_gets_no_half_credit_for_a_slip() {
        assert_eq!(score_gap_fill(&s(&["tre"]), &[s(&["tree"])]).score, 0.0);
        assert_eq!(score_gap_fill(&s(&["teh"]), &[s(&["the"])]).score, 0.0);
    }

    #[test]
    fn gap_fill_treats_a_missing_or_blank_answer_as_wrong() {
        let accepted = vec![s(&["am"]), s(&["is"]), s(&["are"])];
        assert_eq!(score_gap_fill(&s(&["am"]), &accepted).score, 1.0 / 3.0);
        assert_eq!(score_gap_fill(&s(&["", "  ", ""]), &accepted).score, 0.0);
        assert_eq!(score_gap_fill(&[], &[]).score, 0.0);
    }

    #[test]
    fn reorder_needs_the_exact_order() {
        let answer = s(&["My", "name", "is", "Dewi"]);
        assert_eq!(
            score_reorder(&s(&["my", "NAME", "is", "dewi"]), &answer),
            1.0
        );
        assert_eq!(
            score_reorder(&s(&["name", "my", "is", "Dewi"]), &answer),
            0.0
        );
        assert_eq!(score_reorder(&s(&["My", "name", "is"]), &answer), 0.0);
    }

    #[test]
    fn match_scores_the_share_of_pairs() {
        let correct = [1, 0, 2];
        assert_eq!(score_match(&[Some(1), Some(0), Some(2)], &correct), 1.0);
        assert!((score_match(&[Some(1), Some(2), None], &correct) - 1.0 / 3.0).abs() < 1e-9);
        assert_eq!(score_match(&[], &correct), 0.0);
        assert_eq!(score_match(&[], &[]), 0.0);
    }

    #[test]
    fn dictation_is_one_minus_the_word_error_rate() {
        let accepted = s(&["Good morning, I'm Dewi."]);
        assert_eq!(score_dictation("good morning i am dewi", &accepted), 1.0);
        // One wrong word out of five reference words once "I'm" is written out: good morning i am dewi.
        let one_wrong = score_dictation("good morning i am dewa", &accepted);
        assert!((one_wrong - 0.8).abs() < 1e-9, "{one_wrong}");
        assert_eq!(score_dictation("", &accepted), 0.0);
    }

    #[test]
    fn dictation_never_goes_below_zero_and_takes_the_best_accepted_answer() {
        let accepted = s(&["see you", "see you later"]);
        assert_eq!(
            score_dictation("completely different words here now", &accepted),
            0.0
        );
        assert_eq!(score_dictation("see you later", &accepted), 1.0);
        assert_eq!(score_dictation("anything", &[]), 0.0);
    }

    #[test]
    fn error_correction_needs_an_accepted_sentence() {
        let accepted = s(&["I am a student.", "I'm a student."]);
        assert_eq!(score_error_correction("i am a student", &accepted), 1.0);
        assert_eq!(score_error_correction("I'm a student!", &accepted), 1.0);
        assert_eq!(score_error_correction("I a student", &accepted), 0.0);
        assert_eq!(score_error_correction("", &accepted), 0.0);
    }
}
