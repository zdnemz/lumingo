//! Per-unit warnings W01 to W07 of `docs/CURRICULUM_SPEC.md` section 6.

use std::collections::{HashMap, HashSet};

use super::report::{Finding, RuleCode, UnitReport};
use super::text::{lowercase_words, word_count, word_set_similarity};
use super::texts::{TextScope, localized_texts, unit_texts};
use super::unit_rules::{listening_range, writing_range};
use crate::model::{Activity, Band, Level, LevelTag, PartOfSpeech, Unit};

/// More than this share of running words above the unit level raises W01.
const ABOVE_LEVEL_SHARE: f64 = 0.05;
/// Word-set similarity from which two activities count as nearly equal for W04.
const NEAR_DUPLICATE_SIMILARITY: f64 = 0.9;
/// W04 ignores texts shorter than this many words, because short stems repeat by design.
const NEAR_DUPLICATE_MIN_WORDS: usize = 4;
/// W05 ignores English texts shorter than this many characters.
const LENGTH_RATIO_MIN_CHARS: usize = 12;

/// A word list with one CEFR level per word, loaded from a file under `curriculum/data/`.
///
/// Text format, one entry per line: the word or phrase, a comma or a tab, the level
/// (`A1` to `C2`). Empty lines and lines starting with `#` are ignored. When a word is listed
/// twice, the lower level wins. The project ships no list yet; each list needs its own
/// license file and a row in `docs/LICENSE_REGISTER.md` first.
#[derive(Debug, Clone, Default)]
pub struct WordLevels {
    levels: HashMap<String, Level>,
}

/// A line of a word list that could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("word list line {line}: {reason}")]
pub struct WordListError {
    pub line: usize,
    pub reason: String,
}

impl WordLevels {
    pub fn parse(text: &str) -> Result<Self, WordListError> {
        let mut levels: HashMap<String, Level> = HashMap::new();
        for (index, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let number = index + 1;
            let Some((word, level)) = line.rsplit_once([',', '\t']) else {
                return Err(WordListError {
                    line: number,
                    reason: "expected a word, a comma or tab, and a level".to_owned(),
                });
            };
            let Some(level) = Level::parse(level.trim()) else {
                return Err(WordListError {
                    line: number,
                    reason: format!("\"{}\" is not a level from A1 to C2", level.trim()),
                });
            };
            let key = word.trim().to_lowercase();
            levels
                .entry(key)
                .and_modify(|known| *known = (*known).min(level))
                .or_insert(level);
        }
        Ok(Self { levels })
    }

    pub fn len(&self) -> usize {
        self.levels.len()
    }

    pub fn is_empty(&self) -> bool {
        self.levels.is_empty()
    }

    /// The list level of `word`, or `None` when no list has it. Plain inflections are tried when
    /// the form itself is missing: a trailing `s`, `es`, `ed` or `ing`.
    pub fn level_of(&self, word: &str) -> Option<Level> {
        let lowered = word.to_lowercase();
        if let Some(level) = self.levels.get(&lowered) {
            return Some(*level);
        }
        ["s", "es", "ed", "ing"].iter().find_map(|suffix| {
            lowered
                .strip_suffix(suffix)
                .filter(|stem| stem.len() >= 3)
                .and_then(|stem| self.levels.get(stem).copied())
        })
    }
}

/// A rule-based grammar checker, used for W03. The Harper checker will implement this once the
/// tutor links it. Each returned string is one finding.
pub trait GrammarCheck {
    fn findings(&self, text: &str) -> Vec<String>;
}

/// What the warning rules may use besides the unit itself.
#[derive(Default, Clone, Copy)]
pub struct UnitOptions<'a> {
    /// Word levels for W01 and W02. Without them both rules are reported as skipped.
    pub word_levels: Option<&'a WordLevels>,
    /// A grammar checker for W03. Without it the rule is reported as skipped.
    pub grammar_check: Option<&'a dyn GrammarCheck>,
}

/// W01 to W07, appended to `report`.
pub fn check_warnings(
    unit: &Unit,
    document: &serde_json::Value,
    options: &UnitOptions<'_>,
    report: &mut UnitReport,
) {
    match options.word_levels {
        Some(levels) => {
            check_vocabulary_profile(unit, levels, report);
            check_level_tags(unit, levels, report);
        }
        None => {
            let reason =
                "no word list was given; pass --word-list with a list from curriculum/data";
            report.skip(RuleCode::W01, reason);
            report.skip(RuleCode::W02, reason);
        }
    }
    match options.grammar_check {
        Some(checker) => check_at_answers(unit, checker, report),
        None => report.skip(
            RuleCode::W03,
            "no rule-based grammar checker is linked into this build",
        ),
    }
    check_near_duplicates(unit, report);
    check_language_balance(document, report);
    check_writing_and_listening_length(unit, report);
}

/// Words that count towards W01: no digits, and no capitalised word inside a sentence (a name).
fn profile_words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
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
                words.extend(lowercase_words(bare));
            }
        }
        sentence_start = ends_sentence;
    }
    words
}

fn check_vocabulary_profile(unit: &Unit, levels: &WordLevels, report: &mut UnitReport) {
    let targets: HashSet<String> = unit
        .targets
        .vocabulary
        .iter()
        .flat_map(|item| lowercase_words(&item.lemma))
        .collect();
    let mut total = 0usize;
    let mut above: Vec<String> = Vec::new();
    for (_, text) in unit_texts(unit, TextScope::Everything) {
        for word in profile_words(text) {
            total += 1;
            let is_above = levels
                .level_of(&word)
                .is_some_and(|level| level > unit.level);
            if is_above && !targets.contains(&word) {
                above.push(word);
            }
        }
    }
    if total == 0 {
        return;
    }
    let share = above.len() as f64 / total as f64;
    if share > ABOVE_LEVEL_SHARE {
        above.sort();
        above.dedup();
        let sample: Vec<&str> = above.iter().take(8).map(String::as_str).collect();
        report.push(Finding::new(
            RuleCode::W01,
            "",
            format!(
                "{:.1} percent of {total} running words are above level {} and not unit targets, for example: {}",
                share * 100.0,
                unit.level,
                sample.join(", ")
            ),
        ));
    }
}

fn check_level_tags(unit: &Unit, levels: &WordLevels, report: &mut UnitReport) {
    for (i, item) in unit.targets.vocabulary.iter().enumerate() {
        let listed = levels.level_of(&item.lemma);
        // A phrase is rarely in a word list, so a missing phrase is not a disagreement.
        if listed.is_none() && (item.pos == PartOfSpeech::Phrase || item.lemma.contains(' ')) {
            continue;
        }
        let expected = listed.map_or(LevelTag::Unlisted, LevelTag::from);
        if item.level_tag != expected {
            report.push(Finding::new(
                RuleCode::W02,
                format!("/targets/vocabulary/{i}/level_tag"),
                format!(
                    "\"{}\" is tagged {:?} but the word list says {:?}",
                    item.lemma, item.level_tag, expected
                ),
            ));
        }
    }
}

fn check_at_answers(unit: &Unit, checker: &dyn GrammarCheck, report: &mut UnitReport) {
    for (i, activity) in unit.activities.iter().enumerate() {
        let answers = match activity {
            Activity::GuidedSpeaking(a) | Activity::GuidedWriting(a) => &a.model_answers,
            Activity::Mediation(a) => &a.model_answers,
            _ => continue,
        };
        for (m, answer) in answers.iter().enumerate() {
            if answer.band != Band::At {
                continue;
            }
            let findings = checker.findings(&answer.text);
            if !findings.is_empty() {
                report.push(Finding::new(
                    RuleCode::W03,
                    format!("/activities/{i}/model_answers/{m}/text"),
                    format!(
                        "the grammar checker reports {} finding(s) in the at answer: {}",
                        findings.len(),
                        findings.join("; ")
                    ),
                ));
            }
        }
    }
}

/// The text W04 compares for an activity, with the field it came from.
fn comparison_text(activity: &Activity) -> Option<(&'static str, &str)> {
    match activity {
        Activity::Mcq(a) => Some(("stem", &a.stem)),
        Activity::GapFill(a) => Some(("text", &a.text)),
        Activity::Reorder(a) => Some(("answer", &a.answer)),
        Activity::Dictation(a) => Some(("audio_text", &a.audio_text)),
        Activity::ReadAloud(a) => Some(("text", &a.text)),
        Activity::GuidedSpeaking(a) | Activity::GuidedWriting(a) => {
            Some(("prompt/en", &a.prompt.en))
        }
        Activity::Roleplay(a) => Some(("scenario/en", &a.scenario.en)),
        Activity::Mediation(a) => Some(("source_text", &a.source_text)),
        Activity::ReadingSet(a) => Some(("passage", &a.passage)),
        Activity::ListeningSet(a) => a.audio_text.as_deref().map(|t| ("audio_text", t)),
        Activity::ErrorCorrection(a) => Some(("sentence", &a.sentence)),
        Activity::Match(_) | Activity::MinimalPairs(_) | Activity::Shadowing(_) => None,
    }
}

fn check_near_duplicates(unit: &Unit, report: &mut UnitReport) {
    let texts: Vec<(usize, &'static str, &str)> = unit
        .activities
        .iter()
        .enumerate()
        .filter_map(|(i, a)| comparison_text(a).map(|(field, text)| (i, field, text)))
        .filter(|(_, _, text)| lowercase_words(text).len() >= NEAR_DUPLICATE_MIN_WORDS)
        .collect();
    for (n, (i, field, text)) in texts.iter().enumerate() {
        for (j, other_field, other) in &texts[n + 1..] {
            if word_set_similarity(text, other) >= NEAR_DUPLICATE_SIMILARITY {
                report.push(Finding::new(
                    RuleCode::W04,
                    format!("/activities/{j}/{other_field}"),
                    format!(
                        "nearly the same text as /activities/{i}/{field} (activities \"{}\" and \"{}\")",
                        unit.activities[*i].id(),
                        unit.activities[*j].id()
                    ),
                ));
            }
        }
    }
}

fn check_language_balance(document: &serde_json::Value, report: &mut UnitReport) {
    for text in localized_texts(document) {
        let Some(indonesian) = text.id else { continue };
        let english_len = text.en.chars().count();
        let indonesian_len = indonesian.chars().count();
        if english_len < LENGTH_RATIO_MIN_CHARS {
            continue;
        }
        if indonesian_len > english_len * 2 {
            report.push(Finding::new(
                RuleCode::W05,
                format!("{}/id", text.pointer),
                format!(
                    "the Indonesian text has {indonesian_len} characters, more than twice the {english_len} of the English text"
                ),
            ));
        } else if indonesian_len * 2 < english_len {
            report.push(Finding::new(
                RuleCode::W05,
                format!("{}/id", text.pointer),
                format!(
                    "the Indonesian text has {indonesian_len} characters, less than half the {english_len} of the English text"
                ),
            ));
        }
    }
}

fn check_writing_and_listening_length(unit: &Unit, report: &mut UnitReport) {
    let (write_min, write_max) = writing_range(unit.level);
    let (listen_min, listen_max) = listening_range(unit.level);
    for (i, activity) in unit.activities.iter().enumerate() {
        match activity {
            Activity::GuidedWriting(a) => {
                if let Some((m, at)) = a
                    .model_answers
                    .iter()
                    .enumerate()
                    .find(|(_, m)| m.band == Band::At)
                {
                    let words = word_count(&at.text);
                    if words < write_min || words > write_max {
                        report.push(Finding::new(
                            RuleCode::W06,
                            format!("/activities/{i}/model_answers/{m}/text"),
                            format!(
                                "the at answer has {words} words, {} writing tasks aim at {write_min} to {write_max}",
                                unit.level
                            ),
                        ));
                    }
                }
            }
            Activity::ListeningSet(a) => {
                let words = match (&a.audio_text, &a.dialogue_id) {
                    (Some(text), _) => Some(word_count(text)),
                    (None, Some(id)) => unit
                        .dialogues
                        .iter()
                        .find(|d| &d.id == id)
                        .map(|d| d.turns.iter().map(|t| word_count(&t.text)).sum()),
                    (None, None) => None,
                };
                let outside = words.filter(|w| *w < listen_min || *w > listen_max);
                if let Some(words) = outside {
                    report.push(Finding::new(
                        RuleCode::W07,
                        format!("/activities/{i}"),
                        format!(
                            "the audio text has {words} words, {} listening sets have {listen_min} to {listen_max}",
                            unit.level
                        ),
                    ));
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_list_parses_levels_and_keeps_the_lowest() {
        let list =
            WordLevels::parse("# comment\nhello,A1\nthank you\tA1\nhello,B2\n\ndiligent, C1\n")
                .unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list.level_of("Hello"), Some(Level::A1));
        assert_eq!(list.level_of("thank you"), Some(Level::A1));
        assert_eq!(list.level_of("diligent"), Some(Level::C1));
        assert_eq!(list.level_of("zebra"), None);
    }

    #[test]
    fn word_list_rejects_bad_lines() {
        assert_eq!(WordLevels::parse("hello A1").unwrap_err().line, 1);
        assert_eq!(WordLevels::parse("ok,A1\nbad,Z9").unwrap_err().line, 2);
    }

    #[test]
    fn inflections_fall_back_to_the_stem() {
        let list = WordLevels::parse("book,A1\nwalk,A1").unwrap();
        assert_eq!(list.level_of("books"), Some(Level::A1));
        assert_eq!(list.level_of("walking"), Some(Level::A1));
        assert_eq!(list.level_of("walked"), Some(Level::A1));
    }

    #[test]
    fn profile_words_skip_names_and_digits() {
        assert_eq!(
            profile_words("Hello! I am Dewi from Bandung. Room 12 is nice."),
            ["hello", "i", "am", "from", "room", "is", "nice"]
        );
    }
}
