//! Comparing a revised draft with the first one in the writing workshop.
//!
//! An earlier error is *fixed* when its quoted text is gone from the new draft
//! and the new analysis does not report it again. It is *remaining* when the new
//! analysis reports it again, with the same category on the same quoted words.
//! Any error the new analysis reports that was not in the first one is *new*.

use assessment_engine::normalize;
use serde::{Deserialize, Serialize};

/// One error found in a draft: the words in the learner's text, and its category.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftError {
    /// The exact words from the draft, as quoted by the analysis.
    pub quote: String,
    /// The error category from the error catalog, for example `verb_tense`.
    pub category: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    Fixed,
    Remaining,
}

/// What happened to one error of the first draft.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EarlierError {
    pub error: DraftError,
    pub resolution: Resolution,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftComparison {
    pub earlier: Vec<EarlierError>,
    pub new: Vec<DraftError>,
}

impl DraftComparison {
    pub fn fixed(&self) -> usize {
        self.earlier
            .iter()
            .filter(|e| e.resolution == Resolution::Fixed)
            .count()
    }
    pub fn remaining(&self) -> usize {
        self.earlier
            .iter()
            .filter(|e| e.resolution == Resolution::Remaining)
            .count()
    }
}

/// The same words and category, ignoring case, spacing and final punctuation.
fn same_error(a: &DraftError, b: &DraftError) -> bool {
    a.category == b.category && normalize(&a.quote) == normalize(&b.quote)
}

/// Compares the first draft's errors with the analysis of the second draft.
///
/// `second_text` is the revised draft itself. A quote that no longer appears in
/// it cannot still be an error, so a re-reported error whose words are gone is
/// treated as new rather than remaining.
pub fn compare_drafts(
    first: &[DraftError],
    second_text: &str,
    second: &[DraftError],
) -> DraftComparison {
    let text = normalize(second_text);
    let present = |error: &DraftError| {
        let quote = normalize(&error.quote);
        !quote.is_empty() && text.contains(&quote)
    };

    let earlier = first
        .iter()
        .map(|error| {
            let reported_again = second.iter().any(|candidate| same_error(error, candidate));
            let resolution = if reported_again && present(error) {
                Resolution::Remaining
            } else {
                Resolution::Fixed
            };
            EarlierError {
                error: error.clone(),
                resolution,
            }
        })
        .collect();

    let new = second
        .iter()
        .filter(|candidate| {
            let was_there =
                first.iter().any(|error| same_error(error, candidate)) && present(candidate);
            !was_there
        })
        .cloned()
        .collect();

    DraftComparison { earlier, new }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(quote: &str, category: &str) -> DraftError {
        DraftError {
            quote: quote.into(),
            category: category.into(),
        }
    }

    #[test]
    fn an_error_whose_words_are_gone_and_not_reported_is_fixed() {
        let first = [err("I go to the market yesterday", "verb_tense")];
        let result = compare_drafts(&first, "Yesterday I went to the market.", &[]);
        assert_eq!(result.fixed(), 1);
        assert_eq!(result.remaining(), 0);
        assert!(result.new.is_empty());
    }

    #[test]
    fn an_error_reported_again_on_the_same_words_remains() {
        let first = [err("She have two brother", "subject_verb_agreement")];
        let second = [err("she have two brother", "subject_verb_agreement")];
        let result = compare_drafts(&first, "She have two brother. They are kind.", &second);
        assert_eq!(result.remaining(), 1);
        assert_eq!(result.fixed(), 0);
        assert!(result.new.is_empty(), "the same error is not also new");
    }

    #[test]
    fn something_reported_that_was_not_there_before_is_new() {
        let first = [err("I go", "verb_tense")];
        let second = [err("two brothers is", "subject_verb_agreement")];
        let result = compare_drafts(&first, "I went. I have two brothers is kind.", &second);
        assert_eq!(result.fixed(), 1);
        assert_eq!(result.new, second);
    }

    #[test]
    fn the_same_words_with_a_different_category_are_a_different_error() {
        let first = [err("in the morning", "preposition")];
        let second = [err("in the morning", "word_order")];
        let result = compare_drafts(&first, "I wake in the morning.", &second);
        assert_eq!(result.fixed(), 1);
        assert_eq!(result.new.len(), 1);
    }

    #[test]
    fn a_reported_error_whose_words_vanished_is_not_counted_as_remaining() {
        // The analysis repeats an old report, but the learner already rewrote those words.
        let first = [err("I am agree", "verb_form")];
        let second = [err("I am agree", "verb_form")];
        let result = compare_drafts(&first, "I agree with you.", &second);
        assert_eq!(result.remaining(), 0);
        assert_eq!(result.fixed(), 1);
        assert_eq!(
            result.new.len(),
            1,
            "a report about words that are not in the draft is surfaced, not hidden"
        );
    }

    #[test]
    fn matching_ignores_case_spacing_and_final_punctuation() {
        let first = [err("He  don't like", "verb_form")];
        let second = [err("he don't like.", "verb_form")];
        let result = compare_drafts(&first, "he don't   like it", &second);
        assert_eq!(result.remaining(), 1);
    }

    #[test]
    fn with_no_errors_in_either_draft_there_is_nothing_to_report() {
        let result = compare_drafts(&[], "A clean text.", &[]);
        assert_eq!(
            (result.fixed(), result.remaining(), result.new.len()),
            (0, 0, 0)
        );
    }

    #[test]
    fn every_earlier_error_gets_exactly_one_resolution_in_order() {
        let first = [err("a b", "x"), err("c d", "y"), err("e f", "z")];
        let second = [err("c d", "y")];
        let result = compare_drafts(&first, "c d and more", &second);
        let resolutions: Vec<Resolution> = result.earlier.iter().map(|e| e.resolution).collect();
        assert_eq!(
            resolutions,
            [Resolution::Fixed, Resolution::Remaining, Resolution::Fixed]
        );
        assert_eq!(result.earlier.len(), first.len());
    }

    #[test]
    fn an_empty_quote_never_counts_as_present() {
        let first = [err("", "x")];
        let result = compare_drafts(&first, "anything", &[err("", "x")]);
        assert_eq!(result.remaining(), 0);
    }
}
