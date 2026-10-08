//! Text and timing metrics (assessment spec, sections 5.3 and 8).
//!
//! Pure functions over plain data: word counts, the vocabulary profile of a
//! response against a word list the caller supplies, and timing figures from
//! voice-activity data. The rubric cross-checks X2, X4 and X5 read these figures
//! as alarms and caps; none of them is a score, and none of them feeds a level
//! estimate. Timing in particular is descriptive only in v1, because no validated
//! mapping from these numbers to levels exists (spec section 8).
//!
//! The crate ships no word list: each list needs its own license file and a row
//! in the license register first, so the caller owns the data and passes it in
//! through [`WordList`].

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::types::Level;

/// Words, distinct words and sentences of a text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextCounts {
    pub words: usize,
    /// Distinct word forms, case folded. There is no lemmatiser in the project:
    /// "walk" and "walked" count as two. The vocabulary profile groups a few
    /// plain inflections through the word list instead.
    pub distinct_words: usize,
    pub sentences: usize,
}

/// The lower-case words of `text`. A word is a run of letters and digits; an
/// apostrophe or a hyphen between two of them stays inside the word ("don't",
/// "well-known"), and curly apostrophes count as straight ones.
pub fn words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut pending_joiner: Option<char> = None;
    for c in text.chars() {
        let c = if matches!(c, '\u{2019}' | '\u{2018}' | '\u{201b}') {
            '\''
        } else {
            c
        };
        if c.is_alphanumeric() {
            if let Some(joiner) = pending_joiner.take() {
                current.push(joiner);
            }
            current.extend(c.to_lowercase());
        } else if (c == '\'' || c == '-') && !current.is_empty() && pending_joiner.is_none() {
            pending_joiner = Some(c);
        } else {
            pending_joiner = None;
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Number of words, as [`words`] counts them.
pub fn word_count(text: &str) -> usize {
    words(text).len()
}

/// Word, distinct-word and sentence counts. A sentence ends at `.`, `!` or `?`
/// followed by whitespace or the end of the text; text with words and no final
/// mark still counts its last sentence, because a voice transcript has no marks.
pub fn text_counts(text: &str) -> TextCounts {
    let all = words(text);
    let distinct: HashSet<&String> = all.iter().collect();
    let mut sentences = 0;
    let mut open = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_alphanumeric() {
            open = true;
        } else if matches!(c, '.' | '!' | '?')
            && open
            && chars.peek().is_none_or(|n| n.is_whitespace())
        {
            sentences += 1;
            open = false;
        }
    }
    if open {
        sentences += 1;
    }
    TextCounts {
        words: all.len(),
        distinct_words: distinct.len(),
        sentences,
    }
}

/// A word list with one CEFR level per word, supplied by the caller.
pub trait WordList {
    /// The list level of `word` (already lower case), or `None` when no list has it.
    fn level_of(&self, word: &str) -> Option<Level>;
}

impl WordList for HashMap<String, Level> {
    fn level_of(&self, word: &str) -> Option<Level> {
        self.get(word).copied()
    }
}

/// Counts of the running words of a response by their level against the level
/// the task targets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VocabularyProfile {
    /// The level the task targets, the reference for below, at and above.
    pub target: Level,
    /// Running words that were looked up. Numbers are left out, and so are
    /// names (a capitalised word inside a sentence), which no list holds.
    pub considered: usize,
    pub below: usize,
    pub at: usize,
    pub above: usize,
    /// Considered words that no list has.
    pub unlisted: usize,
    /// Distinct considered word forms.
    pub distinct: usize,
    /// Distinct listed word forms above the target level.
    pub distinct_above: usize,
}

impl VocabularyProfile {
    fn share(&self, count: usize) -> f64 {
        if self.considered == 0 {
            0.0
        } else {
            count as f64 / self.considered as f64
        }
    }

    /// Share of considered words below the target level.
    pub fn below_share(&self) -> f64 {
        self.share(self.below)
    }

    /// Share of considered words at the target level.
    pub fn at_share(&self) -> f64 {
        self.share(self.at)
    }

    /// Share of considered words above the target level, the figure of cross-check X4.
    pub fn above_share(&self) -> f64 {
        self.share(self.above)
    }

    /// Share of considered words that no list has.
    pub fn unlisted_share(&self) -> f64 {
        self.share(self.unlisted)
    }

    /// True when the list knew too little of the response to say anything: no
    /// considered word, or every one of them unlisted.
    pub fn is_uninformative(&self) -> bool {
        self.considered == self.unlisted
    }
}

/// Plain inflections tried when a form is missing from the list, the same ones
/// the content validators use: a trailing `s`, `es`, `ed` or `ing`.
fn listed_level(list: &dyn WordList, word: &str) -> Option<Level> {
    if let Some(level) = list.level_of(word) {
        return Some(level);
    }
    ["s", "es", "ed", "ing"].iter().find_map(|suffix| {
        word.strip_suffix(suffix)
            .filter(|stem| stem.chars().count() >= 3)
            .and_then(|stem| list.level_of(stem))
    })
}

/// The words that count for the profile: no number, and no capitalised word
/// inside a sentence (a name). The first word of each sentence is kept, and so
/// is "I". Text without any capital letter, such as a speech transcript, keeps
/// every word.
fn profile_words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut sentence_start = true;
    for piece in text.split_whitespace() {
        let bare = piece.trim_matches(|c: char| !(c.is_alphanumeric() || c == '\''));
        let ends_sentence = piece.ends_with(['.', '?', '!', ':']);
        if !bare.is_empty() {
            let capitalised = bare.chars().next().is_some_and(char::is_uppercase);
            let has_digit = bare.chars().any(|c| c.is_ascii_digit());
            let name_like =
                capitalised && !sentence_start && bare != "I" && !bare.starts_with("I'");
            if !has_digit && !name_like {
                out.extend(words(bare));
            }
        }
        sentence_start = ends_sentence;
    }
    out
}

/// The vocabulary profile of `text` against `list`, for a task at `target`.
pub fn vocabulary_profile(text: &str, target: Level, list: &dyn WordList) -> VocabularyProfile {
    let considered_words = profile_words(text);
    let mut profile = VocabularyProfile {
        target,
        considered: considered_words.len(),
        below: 0,
        at: 0,
        above: 0,
        unlisted: 0,
        distinct: 0,
        distinct_above: 0,
    };
    let mut seen: HashSet<&str> = HashSet::new();
    let mut seen_above: HashSet<&str> = HashSet::new();
    for word in &considered_words {
        let fresh = seen.insert(word.as_str());
        if fresh {
            profile.distinct += 1;
        }
        match listed_level(list, word) {
            None => profile.unlisted += 1,
            Some(level) if level < target => profile.below += 1,
            Some(level) if level == target => profile.at += 1,
            Some(_) => {
                profile.above += 1;
                if seen_above.insert(word.as_str()) {
                    profile.distinct_above += 1;
                }
            }
        }
    }
    profile
}

/// Gaps between voiced stretches this long or longer count as pauses (spec
/// section 8: the mean run between pauses longer than 250 ms).
pub const PAUSE_THRESHOLD_MS: u32 = 250;

/// Most voiced spans one call accepts. A turn of a minute holds a few hundred
/// at most; the cap keeps a faulty detector from making the call unbounded, and
/// longer input is refused, not cut.
pub const MAX_SPANS: usize = 10_000;

/// One voiced stretch of a turn from the voice-activity detector, in
/// milliseconds from the start of the recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoicedSpan {
    pub start_ms: u32,
    pub end_ms: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MetricsError {
    /// A span ends before it starts.
    #[error("voiced span {index} ends before it starts")]
    BackwardsSpan { index: usize },
    /// A span starts before the previous one ended: the list must be sorted and
    /// must not overlap.
    #[error("voiced span {index} starts before the previous span ends")]
    Unordered { index: usize },
    /// More than [`MAX_SPANS`] spans.
    #[error("{count} voiced spans is more than the {MAX_SPANS} one call accepts")]
    TooManySpans { count: usize },
}

/// Timing figures of one voiced turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimingMetrics {
    /// Sum of the voiced spans.
    pub voiced_ms: u64,
    /// First voiced start to last voiced end. Leading and trailing silence are
    /// not part of it: the learner may have waited before speaking.
    pub span_ms: u64,
    /// Silence between voiced spans, all gaps added.
    pub pause_ms: u64,
    /// `pause_ms / span_ms`, 0 for a turn with no span.
    pub pause_ratio: f64,
    /// Gaps of at least [`PAUSE_THRESHOLD_MS`].
    pub pauses: usize,
    /// The longest gap between two voiced spans, 0 when there is none.
    pub longest_pause_ms: u32,
    /// Words per minute of voiced time, `None` with no voiced time.
    pub speech_rate_wpm: Option<f64>,
    /// Mean length of the runs that pauses of at least the threshold separate,
    /// `None` with no span.
    pub mean_run_ms: Option<f64>,
}

/// Timing metrics from the voiced spans of a turn and its word count.
pub fn timing_metrics(spans: &[VoicedSpan], words: usize) -> Result<TimingMetrics, MetricsError> {
    if spans.len() > MAX_SPANS {
        return Err(MetricsError::TooManySpans { count: spans.len() });
    }
    let mut previous_end: Option<u32> = None;
    for (index, span) in spans.iter().enumerate() {
        if span.end_ms < span.start_ms {
            return Err(MetricsError::BackwardsSpan { index });
        }
        if previous_end.is_some_and(|end| span.start_ms < end) {
            return Err(MetricsError::Unordered { index });
        }
        previous_end = Some(span.end_ms);
    }
    let (Some(first), Some(last)) = (spans.first(), spans.last()) else {
        return Ok(TimingMetrics {
            voiced_ms: 0,
            span_ms: 0,
            pause_ms: 0,
            pause_ratio: 0.0,
            pauses: 0,
            longest_pause_ms: 0,
            speech_rate_wpm: None,
            mean_run_ms: None,
        });
    };
    let voiced_ms: u64 = spans.iter().map(|s| u64::from(s.end_ms - s.start_ms)).sum();
    let span_ms = u64::from(last.end_ms - first.start_ms);
    let mut pause_ms = 0_u64;
    let mut pauses = 0;
    let mut longest = 0_u32;
    let mut runs: Vec<u64> = Vec::new();
    let mut run_start = first.start_ms;
    for pair in spans.windows(2) {
        let gap = pair[1].start_ms - pair[0].end_ms;
        pause_ms += u64::from(gap);
        longest = longest.max(gap);
        if gap >= PAUSE_THRESHOLD_MS {
            pauses += 1;
            runs.push(u64::from(pair[0].end_ms - run_start));
            run_start = pair[1].start_ms;
        }
    }
    runs.push(u64::from(last.end_ms - run_start));
    let rate = (voiced_ms > 0).then(|| words as f64 / (voiced_ms as f64 / 60_000.0));
    Ok(TimingMetrics {
        voiced_ms,
        span_ms,
        pause_ms,
        pause_ratio: if span_ms == 0 {
            0.0
        } else {
            pause_ms as f64 / span_ms as f64
        },
        pauses,
        longest_pause_ms: longest,
        speech_rate_wpm: rate,
        mean_run_ms: Some(runs.iter().sum::<u64>() as f64 / runs.len() as f64),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list() -> HashMap<String, Level> {
        [
            ("hello", Level::A1),
            ("my", Level::A1),
            ("name", Level::A1),
            ("is", Level::A1),
            ("from", Level::A1),
            ("like", Level::A1),
            ("cook", Level::A1),
            ("weekend", Level::A2),
            ("friend", Level::A1),
            ("enjoy", Level::B1),
            ("delicious", Level::B2),
            ("meticulous", Level::C1),
        ]
        .into_iter()
        .map(|(w, l)| (w.to_owned(), l))
        .collect()
    }

    #[test]
    fn words_keep_inner_apostrophes_and_hyphens_and_drop_the_rest() {
        assert_eq!(
            words("Hello, I'm Dewi! It\u{2019}s a well-known place -- really."),
            [
                "hello",
                "i'm",
                "dewi",
                "it's",
                "a",
                "well-known",
                "place",
                "really"
            ]
        );
        assert_eq!(words("'quoted' and -dash-"), ["quoted", "and", "dash"]);
        assert!(words("  ... !? ").is_empty());
        assert_eq!(word_count("one two  three"), 3);
    }

    #[test]
    fn counts_cover_words_distinct_words_and_sentences() {
        let counts = text_counts("I like tea. I like cake! Do you like tea?");
        assert_eq!(counts.words, 10);
        assert_eq!(counts.distinct_words, 6);
        assert_eq!(counts.sentences, 3);
        // A transcript has no marks: the one sentence still counts.
        assert_eq!(text_counts("i like tea and cake").sentences, 1);
        // A decimal point does not end a sentence.
        assert_eq!(text_counts("It costs 3.5 dollars.").sentences, 1);
        let empty = text_counts("");
        assert_eq!(
            (empty.words, empty.distinct_words, empty.sentences),
            (0, 0, 0)
        );
    }

    #[test]
    fn the_profile_splits_running_words_by_level_against_the_target() {
        let profile = vocabulary_profile(
            "Hello, my name is Dewi. I enjoy delicious cook.",
            Level::A2,
            &list(),
        );
        // "Dewi" is a name and is left out. "I" is on no list.
        assert_eq!(profile.considered, 8);
        assert_eq!((profile.below, profile.at, profile.above), (5, 0, 2));
        assert_eq!(profile.unlisted, 1);
        assert_eq!(profile.distinct_above, 2);
        assert!((profile.above_share() - 0.25).abs() < 1e-9);
        assert!((profile.below_share() - 0.625).abs() < 1e-9);
        assert!((profile.unlisted_share() - 0.125).abs() < 1e-9);
        assert_eq!(profile.target, Level::A2);
    }

    #[test]
    fn a_word_at_the_target_level_counts_as_at() {
        let profile = vocabulary_profile("my weekend", Level::A2, &list());
        assert_eq!((profile.below, profile.at, profile.above), (1, 1, 0));
    }

    #[test]
    fn plain_inflections_use_the_level_of_their_stem() {
        let profile = vocabulary_profile("friends cooked enjoying", Level::A2, &list());
        assert_eq!((profile.below, profile.above, profile.unlisted), (2, 1, 0));
    }

    #[test]
    fn numbers_and_names_inside_a_sentence_are_left_out_but_a_transcript_keeps_everything() {
        let written = vocabulary_profile("We met Budi in 2024.", Level::A1, &list());
        assert_eq!(written.considered, 3);
        let spoken = vocabulary_profile("we met budi in 2024", Level::A1, &list());
        assert_eq!(
            spoken.considered, 4,
            "no capitals, so nothing looks like a name; the number is left out"
        );
    }

    #[test]
    fn an_empty_text_or_an_empty_list_is_uninformative_and_never_divides_by_zero() {
        let empty = vocabulary_profile("", Level::A1, &list());
        assert_eq!(empty.considered, 0);
        assert_eq!(empty.above_share(), 0.0);
        assert!(empty.is_uninformative());
        let none: HashMap<String, Level> = HashMap::new();
        let unknown = vocabulary_profile("hello there", Level::A1, &none);
        assert!(unknown.is_uninformative());
        assert_eq!(unknown.unlisted, 2);
        let known = vocabulary_profile("hello there", Level::A1, &list());
        assert!(!known.is_uninformative());
    }

    #[test]
    fn the_profile_rises_from_a_weak_to_a_rich_answer() {
        let weak = "I like cook. My friend like cook.";
        let rich = "At the weekend I enjoy a delicious meal with my friend, and we cook together.";
        let richest = "At the weekend I enjoy a delicious and meticulous meal with my friend.";
        let w = vocabulary_profile(weak, Level::A2, &list());
        let r = vocabulary_profile(rich, Level::A2, &list());
        let rr = vocabulary_profile(richest, Level::A2, &list());
        assert_eq!(w.above, 0);
        assert!(r.above >= 1 && rr.above > r.above);
        assert!(w.distinct < r.distinct);
    }

    fn span(start_ms: u32, end_ms: u32) -> VoicedSpan {
        VoicedSpan { start_ms, end_ms }
    }

    #[test]
    fn timing_reports_rate_pauses_and_the_longest_pause() {
        // Three voiced stretches: 0-2000, 2100-4000 (a 100 ms gap), 5000-6000 (a 1000 ms pause).
        let metrics =
            timing_metrics(&[span(0, 2000), span(2100, 4000), span(5000, 6000)], 14).unwrap();
        assert_eq!(metrics.voiced_ms, 4900);
        assert_eq!(metrics.span_ms, 6000);
        assert_eq!(metrics.pause_ms, 1100);
        assert!((metrics.pause_ratio - 1100.0 / 6000.0).abs() < 1e-9);
        assert_eq!(metrics.pauses, 1, "only the 1000 ms gap is a pause");
        assert_eq!(metrics.longest_pause_ms, 1000);
        let wpm = metrics.speech_rate_wpm.unwrap();
        assert!((wpm - 14.0 / (4900.0 / 60_000.0)).abs() < 1e-9);
        // Runs: 0-4000 and 5000-6000.
        assert!((metrics.mean_run_ms.unwrap() - 2500.0).abs() < 1e-9);
    }

    #[test]
    fn a_gap_of_exactly_the_threshold_is_a_pause_and_one_below_is_not() {
        let at = timing_metrics(&[span(0, 1000), span(1250, 2000)], 4).unwrap();
        assert_eq!(at.pauses, 1);
        let below = timing_metrics(&[span(0, 1000), span(1249, 2000)], 4).unwrap();
        assert_eq!(below.pauses, 0);
        assert_eq!(below.mean_run_ms, Some(2000.0));
    }

    #[test]
    fn no_span_gives_zeros_and_no_rate() {
        let metrics = timing_metrics(&[], 5).unwrap();
        assert_eq!(
            (metrics.voiced_ms, metrics.pauses, metrics.longest_pause_ms),
            (0, 0, 0)
        );
        assert_eq!(metrics.speech_rate_wpm, None);
        assert_eq!(metrics.mean_run_ms, None);
        assert_eq!(metrics.pause_ratio, 0.0);
    }

    #[test]
    fn one_span_has_no_pause_and_a_single_run() {
        let metrics = timing_metrics(&[span(500, 3500)], 6).unwrap();
        assert_eq!(
            (metrics.pauses, metrics.pause_ms, metrics.longest_pause_ms),
            (0, 0, 0)
        );
        assert_eq!(metrics.span_ms, 3000);
        assert_eq!(metrics.mean_run_ms, Some(3000.0));
        assert!((metrics.speech_rate_wpm.unwrap() - 120.0).abs() < 1e-9);
    }

    #[test]
    fn malformed_spans_are_refused() {
        assert_eq!(
            timing_metrics(&[span(10, 5)], 1),
            Err(MetricsError::BackwardsSpan { index: 0 })
        );
        assert_eq!(
            timing_metrics(&[span(0, 100), span(50, 200)], 1),
            Err(MetricsError::Unordered { index: 1 })
        );
        let many: Vec<VoicedSpan> = (0..=MAX_SPANS as u32)
            .map(|i| span(i * 10, i * 10 + 5))
            .collect();
        assert_eq!(
            timing_metrics(&many, 1),
            Err(MetricsError::TooManySpans {
                count: MAX_SPANS + 1
            })
        );
        // Touching spans are fine.
        assert!(timing_metrics(&[span(0, 100), span(100, 200)], 1).is_ok());
    }
}
