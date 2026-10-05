//! Reference text to words and candidate pronunciations (ASSESSMENT_SPEC 7.1
//! step 1).
//!
//! Anything the lexicon cannot pronounce is listed as "not checked" with a
//! reason. It is never guessed and never scored.

use serde::{Deserialize, Serialize};

use crate::arpabet::Phone;
use crate::lexicon::Lexicon;

/// Why a word is not checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotChecked {
    /// The word is not in the lexicon (a name, a rare word, a misspelling).
    NotInLexicon,
    /// The token contains a digit. Numbers are not expanded to words because
    /// the reading ("1998", "3.5") depends on context the engine does not have.
    Number,
    /// The token contains a symbol such as `&` or `%` whose spoken form depends
    /// on context.
    Symbol,
}

/// One word of the reference text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceWord {
    /// The word as normalised: lower case, outer punctuation removed.
    pub text: String,
    pub status: WordStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WordStatus {
    /// Candidate pronunciations in dictionary order. Never empty.
    Checked {
        pronunciations: Vec<Vec<Phone>>,
    },
    NotChecked {
        reason: NotChecked,
    },
}

/// The normalised reference text.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Reference {
    pub words: Vec<ReferenceWord>,
}

impl Reference {
    /// Normalises `text`, splits it into words and looks each one up.
    ///
    /// Rules, all of them visible in the tests:
    /// - case is folded and curly apostrophes become `'`;
    /// - whitespace and dashes (hyphen, en dash, em dash) separate words;
    /// - punctuation is dropped, apostrophes inside a word are kept;
    /// - a word is tried as written (CMUdict has `'cause`), then without outer
    ///   apostrophes (a quoted `'hello'`).
    pub fn from_text(text: &str, lexicon: &Lexicon) -> Reference {
        let mut words = Vec::new();
        for chunk in text.split_whitespace() {
            for part in chunk.split(['-', '\u{2010}', '\u{2011}', '\u{2013}', '\u{2014}']) {
                if let Some(word) = normalise_part(part, lexicon) {
                    words.push(word);
                }
            }
        }
        Reference { words }
    }

    /// Words that were looked up and found.
    pub fn checked_count(&self) -> usize {
        self.words
            .iter()
            .filter(|w| matches!(w.status, WordStatus::Checked { .. }))
            .count()
    }

    /// The words that will not be scored, with the reason for each.
    pub fn not_checked(&self) -> Vec<(&str, NotChecked)> {
        self.words
            .iter()
            .filter_map(|w| match w.status {
                WordStatus::NotChecked { reason } => Some((w.text.as_str(), reason)),
                WordStatus::Checked { .. } => None,
            })
            .collect()
    }
}

const SYMBOLS: [char; 8] = ['&', '%', '$', '+', '=', '@', '#', '*'];

fn is_apostrophe(c: char) -> bool {
    matches!(c, '\'' | '\u{2019}' | '\u{2018}' | '\u{02BC}')
}

fn normalise_part(part: &str, lexicon: &Lexicon) -> Option<ReferenceWord> {
    let cleaned: String = part
        .chars()
        .filter(|c| c.is_alphanumeric() || is_apostrophe(*c) || SYMBOLS.contains(c))
        .map(|c| if is_apostrophe(c) { '\'' } else { c })
        .flat_map(char::to_lowercase)
        .collect();
    if cleaned.is_empty() {
        return None;
    }
    let has_letter = cleaned.chars().any(char::is_alphabetic);
    let has_symbol = cleaned.chars().any(|c| SYMBOLS.contains(&c));
    let not_checked = |reason| {
        Some(ReferenceWord {
            text: cleaned.trim_matches('\'').to_owned(),
            status: WordStatus::NotChecked { reason },
        })
    };
    if cleaned.chars().any(char::is_numeric) {
        return not_checked(NotChecked::Number);
    }
    if has_symbol {
        return not_checked(NotChecked::Symbol);
    }
    if !has_letter {
        // Only apostrophes were left: stray punctuation, not a word.
        return None;
    }
    let candidates = [cleaned.as_str(), cleaned.trim_matches('\'')];
    for candidate in candidates {
        if let Some(found) = lexicon.lookup(candidate) {
            return Some(ReferenceWord {
                text: candidate.to_owned(),
                status: WordStatus::Checked {
                    pronunciations: found.to_vec(),
                },
            });
        }
    }
    not_checked(NotChecked::NotInLexicon)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lexicon() -> Lexicon {
        Lexicon::parse(
            "i'm AY1 M\n\
             hello HH AH0 L OW1\n\
             hello(2) HH EH0 L OW1\n\
             world W ER1 L D\n\
             well W EH1 L\n\
             known N OW1 N\n\
             'cause K AH0 Z\n\
             dogs D AO1 G Z\n",
        )
        .expect("parses")
    }

    fn texts(reference: &Reference) -> Vec<&str> {
        reference.words.iter().map(|w| w.text.as_str()).collect()
    }

    #[test]
    fn normalises_case_punctuation_and_curly_apostrophes() {
        let r = Reference::from_text("Hello, WORLD! I\u{2019}m here.", &lexicon());
        assert_eq!(texts(&r), ["hello", "world", "i'm", "here"]);
        assert_eq!(r.checked_count(), 3);
        assert_eq!(r.not_checked(), [("here", NotChecked::NotInLexicon)]);
    }

    #[test]
    fn variants_are_kept() {
        let r = Reference::from_text("hello", &lexicon());
        match &r.words[0].status {
            WordStatus::Checked { pronunciations } => assert_eq!(pronunciations.len(), 2),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn dashes_split_words() {
        let r = Reference::from_text("well-known \u{2014} world", &lexicon());
        assert_eq!(texts(&r), ["well", "known", "world"]);
        assert_eq!(r.checked_count(), 3);
    }

    #[test]
    fn quoted_word_falls_back_to_the_bare_word() {
        let r = Reference::from_text("'hello' 'cause", &lexicon());
        assert_eq!(texts(&r), ["hello", "'cause"]);
        assert_eq!(r.checked_count(), 2);
    }

    #[test]
    fn numbers_and_symbols_are_not_checked_not_guessed() {
        let r = Reference::from_text("call 911 at 3.5% R&B", &lexicon());
        let reasons: Vec<_> = r.not_checked().into_iter().map(|(_, why)| why).collect();
        assert_eq!(
            reasons,
            [
                NotChecked::NotInLexicon,
                NotChecked::Number,
                NotChecked::NotInLexicon,
                NotChecked::Number,
                NotChecked::Symbol
            ]
        );
    }

    #[test]
    fn stray_punctuation_is_not_a_word() {
        let r = Reference::from_text("... -- ' ?! hello", &lexicon());
        assert_eq!(texts(&r), ["hello"]);
    }

    #[test]
    fn empty_text_has_no_words() {
        assert!(Reference::from_text("   ", &lexicon()).words.is_empty());
    }

    #[test]
    fn lookup_is_case_insensitive_through_normalisation() {
        let r = Reference::from_text("DOGS", &lexicon());
        assert_eq!(r.checked_count(), 1);
    }
}
