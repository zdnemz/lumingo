//! Text and timing measurements used as cross-checks for rubric scores
//! (ASSESSMENT_SPEC 5.3, X4 and X5) and for the descriptive fluency trends
//! (section 8). None of these sets a score or a level on its own.

use crate::Level;
use harper_core::{
    Dialect, Document,
    linting::{LintGroup, Linter},
    parsers::PlainEnglish,
    spell::FstDictionary,
};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Running words of `text`, lowercased. Letters count; an apostrophe or hyphen
/// inside a word stays ("don't", "well-known"); digits and punctuation split words.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut flush = |current: &mut String| {
        let word = current.trim_matches(|c| c == '\'' || c == '-').to_owned();
        if word.chars().any(char::is_alphabetic) {
            words.push(word);
        }
        current.clear();
    };
    for c in text.chars() {
        match c {
            c if c.is_alphabetic() => current.extend(c.to_lowercase()),
            '\'' | '\u{2019}' => current.push('\''),
            '-' => current.push('-'),
            _ => flush(&mut current),
        }
    }
    flush(&mut current);
    words
}

pub fn word_count(text: &str) -> usize {
    tokenize(text).len()
}

/// A word list from `curriculum/data/`, mapping a word to the level it is taught at.
/// The project ships no list yet: each one needs a row in the license register first.
#[derive(Debug, Default, Clone)]
pub struct WordList(HashMap<String, Level>);

impl WordList {
    pub fn from_pairs<'a>(pairs: impl IntoIterator<Item = (&'a str, Level)>) -> Self {
        Self(
            pairs
                .into_iter()
                .map(|(w, l)| (w.to_lowercase(), l))
                .collect(),
        )
    }

    /// Looks the word up as written, then with common English endings removed.
    /// ponytail: a suffix stripper, not a lemmatiser. Irregular forms ("went")
    /// are found only when the list contains them. Upgrade when a lemma table is licensed.
    pub fn level_of(&self, word: &str) -> Option<Level> {
        stems(word)
            .into_iter()
            .find_map(|s| self.0.get(&s).copied())
    }
}

fn stems(word: &str) -> Vec<String> {
    let mut out = vec![word.to_owned()];
    let strip = |suffix: &str| {
        word.strip_suffix(suffix)
            .filter(|stem| stem.chars().count() >= 3)
    };
    if let Some(s) = strip("ies") {
        out.push(format!("{s}y"));
    }
    if let Some(s) = strip("ied") {
        out.push(format!("{s}y"));
    }
    for suffix in ["es", "s", "ed", "d", "ing", "ly"] {
        if let Some(s) = strip(suffix) {
            out.push(s.to_owned());
            // "making" -> "make", "liked" is covered by the "d" rule
            if matches!(suffix, "ing" | "ed") {
                out.push(format!("{s}e"));
                let mut chars: Vec<char> = s.chars().collect();
                if chars.len() >= 2 && chars[chars.len() - 1] == chars[chars.len() - 2] {
                    chars.pop();
                    out.push(chars.into_iter().collect()); // "running" -> "run"
                }
            }
        }
    }
    out
}

/// Range of a response measured against a word list (X4).
#[derive(Debug, Clone, PartialEq)]
pub struct VocabularyProfile {
    /// Running words.
    pub words: usize,
    /// Different words as written. This is not a lemma count (see `WordList::level_of`).
    pub distinct: usize,
    /// Running words per level, for words found on the list.
    pub by_level: BTreeMap<Level, usize>,
    /// Running words not on the list: names, rare words, learner errors.
    pub unlisted: usize,
}

impl VocabularyProfile {
    /// Share of all running words that the list places above `level`.
    pub fn share_above(&self, level: Level) -> f64 {
        if self.words == 0 {
            return 0.0;
        }
        self.by_level
            .range(..)
            .filter(|(l, _)| **l > level)
            .map(|(_, n)| n)
            .sum::<usize>() as f64
            / self.words as f64
    }
}

pub fn vocabulary_profile(text: &str, list: &WordList) -> VocabularyProfile {
    let words = tokenize(text);
    let mut by_level = BTreeMap::new();
    let mut unlisted = 0;
    for w in &words {
        match list.level_of(w) {
            Some(level) => *by_level.entry(level).or_insert(0) += 1,
            None => unlisted += 1,
        }
    }
    VocabularyProfile {
        words: words.len(),
        distinct: words.iter().collect::<HashSet<_>>().len(),
        by_level,
        unlisted,
    }
}

/// One rule-based finding. Offsets count characters, not bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub start: usize,
    pub end: usize,
    pub kind: String,
    pub message: String,
    pub suggestions: Vec<String>,
}

/// Rule-based grammar and spelling checker (`harper-core`, no network). Building
/// it loads a dictionary, so create one and reuse it. It is not `Send`: keep it
/// on the thread that created it.
pub struct GrammarChecker {
    linter: LintGroup,
}

impl Default for GrammarChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl GrammarChecker {
    pub fn new() -> Self {
        Self {
            linter: LintGroup::new_curated(FstDictionary::curated(), Dialect::American),
        }
    }

    /// `include_spelling` is false for text that came from speech, where spelling
    /// belongs to the transcriber and not to the learner.
    pub fn findings(&mut self, text: &str, include_spelling: bool) -> Vec<Finding> {
        let document = Document::new_curated(text, &PlainEnglish);
        self.linter
            .lint(&document)
            .into_iter()
            .filter(|l| include_spelling || format!("{:?}", l.lint_kind) != "Spelling")
            .map(|l| Finding {
                start: l.span.start,
                end: l.span.end,
                kind: format!("{:?}", l.lint_kind),
                message: l.message,
                suggestions: l.suggestions.iter().map(ToString::to_string).collect(),
            })
            .collect()
    }
}

/// Findings per 100 running words. Zero words gives zero.
pub fn findings_per_100_words(findings: usize, words: usize) -> f64 {
    if words == 0 {
        0.0
    } else {
        findings as f64 * 100.0 / words as f64
    }
}

/// Speech timing of one voiced learner turn (section 8). Descriptive only: it
/// never counts toward an estimate.
#[derive(Debug, Clone, PartialEq)]
pub struct Fluency {
    /// Words per minute of voiced time.
    pub speech_rate_wpm: f64,
    /// Share of the turn, from first to last voiced sound, that was silent.
    pub pause_ratio: f64,
    /// Mean words between pauses longer than 250 ms. Needs word timings from the STT.
    pub mean_run_words: Option<f64>,
}

const PAUSE_SECONDS: f64 = 0.25;

/// `voiced` holds (start, end) seconds of VAD speech segments in order. `words`
/// is the transcript's word count, and `word_times` (start, end) per word when the
/// STT gives them. `None` when there is no voiced time or no words.
pub fn fluency(
    voiced: &[(f64, f64)],
    words: usize,
    word_times: Option<&[(f64, f64)]>,
) -> Option<Fluency> {
    let (first, last) = (voiced.first()?.0, voiced.last()?.1);
    let voiced_time: f64 = voiced.iter().map(|(s, e)| (e - s).max(0.0)).sum();
    let span = last - first;
    if voiced_time <= 0.0 || span <= 0.0 || words == 0 {
        return None;
    }
    let mean_run_words = word_times.filter(|t| !t.is_empty()).map(|times| {
        let breaks = times
            .windows(2)
            .filter(|w| w[1].0 - w[0].1 > PAUSE_SECONDS)
            .count();
        times.len() as f64 / (breaks + 1) as f64
    });
    Some(Fluency {
        speech_rate_wpm: words as f64 / (voiced_time / 60.0),
        pause_ratio: ((span - voiced_time) / span).clamp(0.0, 1.0),
        mean_run_words,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_keeps_inner_apostrophes_and_hyphens_and_drops_numbers() {
        assert_eq!(
            tokenize("I don't like 3 well-known cats, OK?"),
            ["i", "don't", "like", "well-known", "cats", "ok"]
        );
        assert_eq!(
            tokenize("It\u{2019}s fine -- really"),
            ["it's", "fine", "really"]
        );
        assert_eq!(word_count("  ... 123 "), 0);
    }

    fn list() -> WordList {
        WordList::from_pairs([
            ("go", Level::A1),
            ("book", Level::A1),
            ("run", Level::A1),
            ("make", Level::A1),
            ("study", Level::A2),
            ("achieve", Level::B2),
            ("sophisticated", Level::C1),
        ])
    }

    #[test]
    fn suffix_stripping_finds_common_inflections() {
        let l = list();
        for (word, level) in [
            ("books", Level::A1),
            ("studies", Level::A2),
            ("running", Level::A1),
            ("making", Level::A1),
            ("achieved", Level::B2),
            ("went", Level::A1),
        ] {
            let found = l.level_of(word);
            if word == "went" {
                assert_eq!(found, None, "irregular forms need to be on the list");
            } else {
                assert_eq!(found, Some(level), "{word}");
            }
        }
    }

    #[test]
    fn the_profile_counts_levels_unlisted_words_and_the_share_above_a_level() {
        let p = vocabulary_profile(
            "I go and study books. Sophisticated people achieve Zorblax.",
            &list(),
        );
        assert_eq!((p.words, p.distinct), (9, 9));
        assert_eq!(p.by_level.get(&Level::A1), Some(&2));
        assert_eq!(p.unlisted, 4); // i, and, people, zorblax
        assert!((p.share_above(Level::A2) - 2.0 / 9.0).abs() < 1e-9);
        assert_eq!(p.share_above(Level::C2), 0.0);
        assert_eq!(vocabulary_profile("", &list()).share_above(Level::A1), 0.0);
    }

    #[test]
    fn a_more_advanced_text_has_a_larger_share_above_the_level() {
        let l = list();
        let simple = vocabulary_profile("I go and make books", &l);
        let advanced = vocabulary_profile("I achieve sophisticated books", &l);
        assert!(advanced.share_above(Level::A2) > simple.share_above(Level::A2));
    }

    #[test]
    fn the_checker_flags_a_wrong_article_and_leaves_a_clean_sentence_alone() {
        let mut c = GrammarChecker::new();
        let bad = c.findings("This is an test.", true);
        assert!(!bad.is_empty(), "expected a finding for 'an test'");
        assert!(
            bad.iter()
                .any(|f| f.suggestions.iter().any(|s| s.contains('a'))),
            "{bad:?}"
        );
        assert!(c.findings("She goes to school every day.", true).is_empty());
    }

    #[test]
    fn spelling_findings_can_be_left_out_for_transcribed_speech() {
        let mut c = GrammarChecker::new();
        let with = c.findings("I recieve a letter.", true);
        let without = c.findings("I recieve a letter.", false);
        assert!(with.iter().any(|f| f.kind == "Spelling"), "{with:?}");
        assert!(without.iter().all(|f| f.kind != "Spelling"));
    }

    #[test]
    fn findings_per_100_words_is_a_plain_rate() {
        assert_eq!(findings_per_100_words(3, 60), 5.0);
        assert_eq!(findings_per_100_words(3, 0), 0.0);
    }

    #[test]
    fn fluency_reports_rate_over_voiced_time_pause_ratio_and_run_length() {
        // voiced 0-2 s and 3-5 s: 4 s voiced inside a 5 s span, 12 words
        let f = fluency(&[(0.0, 2.0), (3.0, 5.0)], 12, None).unwrap();
        assert!((f.speech_rate_wpm - 180.0).abs() < 1e-9);
        assert!((f.pause_ratio - 0.2).abs() < 1e-9);
        assert_eq!(f.mean_run_words, None);
        // 6 words with one 400 ms gap and one 100 ms gap: two runs of 3
        let times = [
            (0.0, 0.3),
            (0.3, 0.6),
            (0.6, 0.9),
            (1.3, 1.6),
            (1.7, 2.0),
            (2.0, 2.3),
        ];
        let f = fluency(&[(0.0, 2.3)], 6, Some(&times)).unwrap();
        assert_eq!(f.mean_run_words, Some(3.0));
    }

    #[test]
    fn fluency_needs_voiced_time_and_words() {
        assert_eq!(fluency(&[], 5, None), None);
        assert_eq!(fluency(&[(1.0, 1.0)], 5, None), None);
        assert_eq!(fluency(&[(0.0, 2.0)], 0, None), None);
    }
}
