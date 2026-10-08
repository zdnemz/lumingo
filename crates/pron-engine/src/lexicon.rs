//! A pronunciation lexicon in the `cmudict.dict` text format.
//!
//! The crate ships no dictionary. The loader reads a file the user supplies
//! (CMUdict, for example, which is distributed under its own BSD-style terms).
//! Tests use a small hand-written lexicon that says so in its first lines.
//!
//! Format, one entry per line:
//!
//! ```text
//! word PH PH PH
//! word(2) PH PH PH      <- a second pronunciation of the same word
//! word PH PH # comment
//! ;;; whole-line comment
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::arpabet::{Arpabet, Phone};

/// Why a lexicon could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum LexiconError {
    #[error("cannot read the lexicon file {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("the lexicon file {path} is not valid UTF-8")]
    NotUtf8 { path: PathBuf },
    #[error("lexicon line {line}: {reason}")]
    Parse { line: usize, reason: String },
    #[error("the lexicon holds no entries")]
    Empty,
}

/// A loaded pronunciation lexicon. Lookups are case-insensitive.
#[derive(Debug, Clone, Default)]
pub struct Lexicon {
    entries: HashMap<String, Vec<Vec<Phone>>>,
}

impl Lexicon {
    /// Parses lexicon text. A malformed line is an error, not a skipped line:
    /// a half-read dictionary would silently turn words into "not checked".
    pub fn parse(text: &str) -> Result<Lexicon, LexiconError> {
        let mut entries: HashMap<String, Vec<Vec<Phone>>> = HashMap::new();
        for (index, raw) in text.lines().enumerate() {
            let line = index + 1;
            let body = match raw.split_once('#') {
                Some((before, _)) => before,
                None => raw,
            };
            let body = body.trim();
            if body.is_empty() || body.starts_with(";;;") {
                continue;
            }
            let mut tokens = body.split_whitespace();
            let Some(head) = tokens.next() else { continue };
            let word = strip_variant_marker(head).to_lowercase();
            let mut phones = Vec::new();
            for token in tokens {
                let phone = Phone::parse(token).map_err(|e| LexiconError::Parse {
                    line,
                    reason: e.to_string(),
                })?;
                phones.push(phone);
            }
            if phones.is_empty() {
                return Err(LexiconError::Parse {
                    line,
                    reason: "the entry has no phones".to_owned(),
                });
            }
            let variants = entries.entry(word).or_default();
            // Stress-only duplicates are kept: they are the dictionary's own
            // data. Exact repeats are not.
            if !variants.contains(&phones) {
                variants.push(phones);
            }
        }
        if entries.is_empty() {
            return Err(LexiconError::Empty);
        }
        Ok(Lexicon { entries })
    }

    /// Reads a lexicon file in the format above.
    pub fn from_path(path: &Path) -> Result<Lexicon, LexiconError> {
        let bytes = std::fs::read(path).map_err(|source| LexiconError::Io {
            path: path.to_owned(),
            source,
        })?;
        let text = String::from_utf8(bytes).map_err(|_| LexiconError::NotUtf8 {
            path: path.to_owned(),
        })?;
        Lexicon::parse(&text)
    }

    /// Every pronunciation of `word`, in file order, or `None` when the word is
    /// not in the lexicon.
    pub fn lookup(&self, word: &str) -> Option<&[Vec<Phone>]> {
        let direct = self.entries.get(word);
        match direct {
            Some(v) => Some(v.as_slice()),
            None => self.entries.get(&word.to_lowercase()).map(Vec::as_slice),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Every ARPAbet symbol that occurs in the lexicon. Used by tests and by
    /// the phone-map check to confirm the map covers what the data really uses.
    pub fn symbols_in_use(&self) -> Vec<Arpabet> {
        let mut seen: Vec<Arpabet> = self
            .entries
            .values()
            .flatten()
            .flatten()
            .map(|p| p.symbol)
            .collect();
        seen.sort();
        seen.dedup();
        seen
    }
}

/// `word(2)` becomes `word`. A bracket that is not a variant number stays, so
/// the entry is looked up under its odd spelling and never matches by accident.
fn strip_variant_marker(head: &str) -> &str {
    if let Some(open) = head.rfind('(')
        && let Some(inner) = head[open + 1..].strip_suffix(')')
        && !inner.is_empty()
        && inner.bytes().all(|b| b.is_ascii_digit())
    {
        return &head[..open];
    }
    head
}

#[cfg(test)]
mod tests {
    use super::*;

    fn symbols(variant: &[Phone]) -> Vec<&'static str> {
        variant.iter().map(|p| p.symbol.as_str()).collect()
    }

    #[test]
    fn parses_variants_comments_and_case() {
        let text = ";;; header\n\
                    Read R EH1 D\n\
                    read(2) R IY1 D # present tense\n\
                    \n\
                    the DH AH0\n";
        let lex = Lexicon::parse(text).expect("parses");
        assert_eq!(lex.len(), 2);
        let read = lex.lookup("READ").expect("read is present");
        assert_eq!(read.len(), 2);
        assert_eq!(symbols(&read[0]), ["R", "EH", "D"]);
        assert_eq!(symbols(&read[1]), ["R", "IY", "D"]);
        assert_eq!(read[1][1].stress, Some(1));
    }

    #[test]
    fn repeated_identical_variant_is_stored_once() {
        let lex = Lexicon::parse("a AH0\na(2) AH0\na(3) EY1\n").expect("parses");
        assert_eq!(lex.lookup("a").map(<[_]>::len), Some(2));
    }

    #[test]
    fn unknown_word_is_none() {
        let lex = Lexicon::parse("cat K AE1 T\n").expect("parses");
        assert!(lex.lookup("dog").is_none());
    }

    #[test]
    fn malformed_lines_are_errors_with_the_line_number() {
        let cases = [
            ("cat K AE1 T\ndog D AO9 G\n", 2),
            ("cat K AE1 T\nbird\n", 2),
            ("cat K AE1 T\nfish F IH1 XX\n", 2),
        ];
        for (text, expected_line) in cases {
            match Lexicon::parse(text) {
                Err(LexiconError::Parse { line, .. }) => assert_eq!(line, expected_line),
                other => panic!("expected a parse error, got {other:?}"),
            }
        }
    }

    #[test]
    fn empty_lexicon_is_an_error() {
        assert!(matches!(
            Lexicon::parse(";;; nothing\n\n"),
            Err(LexiconError::Empty)
        ));
    }

    #[test]
    fn missing_file_is_an_io_error() {
        let err = Lexicon::from_path(Path::new("/nonexistent/cmudict.dict"));
        assert!(matches!(err, Err(LexiconError::Io { .. })));
    }

    #[test]
    fn variant_marker_strips_only_digits_in_brackets() {
        assert_eq!(strip_variant_marker("word(12)"), "word");
        assert_eq!(strip_variant_marker("word(a)"), "word(a)");
        assert_eq!(strip_variant_marker("word()"), "word()");
        assert_eq!(strip_variant_marker("word"), "word");
    }
}
