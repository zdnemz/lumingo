//! Text helpers shared by the validators: word counts, normalisation, TTS text rules.

use std::collections::HashSet;

/// Number of words in `text`: whitespace-separated pieces that hold at least one letter or digit.
/// Speaker labels such as `Sari:` count as words, so a passage is measured as it is shown.
pub fn word_count(text: &str) -> usize {
    text.split_whitespace()
        .filter(|piece| piece.chars().any(char::is_alphanumeric))
        .count()
}

/// Lower-case words of `text` for vocabulary and similarity checks. Apostrophes stay inside a
/// word (`what's`), every other mark and the `___` gap marker are dropped.
pub fn lowercase_words(text: &str) -> Vec<String> {
    let without_gaps = text.replace("___", " ");
    without_gaps
        .split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '\u{2019}'))
        .map(|piece| piece.trim_matches(|c| c == '\'' || c == '\u{2019}'))
        .filter(|piece| !piece.is_empty())
        .map(|piece| piece.replace('\u{2019}', "'").to_lowercase())
        .collect()
}

/// Contractions and their full forms. A contraction is rewritten to the full form before two
/// answers are compared. This mirrors the fixed table that `norm/1` of
/// `docs/ASSESSMENT_SPEC.md` section 4 calls for; the assessment engine owns the complete one.
const CONTRACTIONS: [(&str, &str); 34] = [
    ("i'm", "i am"),
    ("you're", "you are"),
    ("he's", "he is"),
    ("she's", "she is"),
    ("it's", "it is"),
    ("we're", "we are"),
    ("they're", "they are"),
    ("that's", "that is"),
    ("what's", "what is"),
    ("where's", "where is"),
    ("who's", "who is"),
    ("there's", "there is"),
    ("here's", "here is"),
    ("don't", "do not"),
    ("doesn't", "does not"),
    ("didn't", "did not"),
    ("can't", "cannot"),
    ("won't", "will not"),
    ("isn't", "is not"),
    ("aren't", "are not"),
    ("wasn't", "was not"),
    ("weren't", "were not"),
    ("haven't", "have not"),
    ("hasn't", "has not"),
    ("wouldn't", "would not"),
    ("couldn't", "could not"),
    ("shouldn't", "should not"),
    ("i've", "i have"),
    ("you've", "you have"),
    ("we've", "we have"),
    ("they've", "they have"),
    ("i'll", "i will"),
    ("you'll", "you will"),
    ("we'll", "we will"),
];

/// Answer normalisation in the spirit of `norm/1`: trim, lower-case, collapse whitespace,
/// straight quotes, drop a final `.`, `?` or `!`, and expand contractions.
pub fn normalise_answer(text: &str) -> String {
    let straight = text
        .replace(['\u{2019}', '\u{2018}'], "'")
        .replace(['\u{201C}', '\u{201D}'], "\"");
    let lowered = straight.trim().to_lowercase();
    let collapsed = lowered.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim_end_matches(['.', '?', '!']).trim_end();
    trimmed
        .split(' ')
        .map(|word| {
            let bare = word.trim_matches(|c: char| !(c.is_alphanumeric() || c == '\''));
            match CONTRACTIONS.iter().find(|(short, _)| *short == bare) {
                Some((_, full)) => word.replacen(bare, full, 1),
                None => word.to_owned(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Splits `text` into sentences at `.`, `?`, `!` and line breaks.
pub fn sentences(text: &str) -> Vec<&str> {
    text.split(['.', '?', '!', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect()
}

/// Jaccard similarity of the word sets of two texts, 0.0 to 1.0.
pub fn word_set_similarity(a: &str, b: &str) -> f64 {
    let set_a: HashSet<String> = lowercase_words(a).into_iter().collect();
    let set_b: HashSet<String> = lowercase_words(b).into_iter().collect();
    if set_a.is_empty() || set_b.is_empty() {
        return 0.0;
    }
    let shared = set_a.intersection(&set_b).count();
    let total = set_a.union(&set_b).count();
    shared as f64 / total as f64
}

/// Abbreviations that a TTS engine reads badly. Rule 14 forbids them.
const SPOKEN_ABBREVIATIONS: [&str; 7] = ["mr", "mrs", "ms", "dr", "prof", "etc", "vs"];

/// Why `text` may not be spoken by TTS, per rule 14: only letters, digits, spaces and the
/// marks `. , ? ! ' -`, and no abbreviations. An empty list means the text is fine.
pub fn tts_problems(text: &str) -> Vec<String> {
    let mut problems = Vec::new();
    let mut bad: Vec<char> = Vec::new();
    for c in text.chars() {
        let allowed = c.is_alphanumeric() || matches!(c, ' ' | '.' | ',' | '?' | '!' | '\'' | '-');
        if !allowed && !bad.contains(&c) {
            bad.push(c);
        }
    }
    if !bad.is_empty() {
        let shown: Vec<String> = bad
            .iter()
            .map(|c| match c {
                '\n' => "a line break".to_owned(),
                '\t' => "a tab".to_owned(),
                other => format!("'{other}'"),
            })
            .collect();
        problems.push(format!(
            "contains {}, but TTS text may only use letters, digits, spaces and . , ? ! ' -",
            shown.join(", ")
        ));
    }

    let chars: Vec<char> = text.chars().collect();
    let mut abbreviation: Option<String> = None;
    for (i, &c) in chars.iter().enumerate() {
        // A letter, a dot and a letter, as in "e.g." or "a.m.".
        if c == '.'
            && i > 0
            && chars[i - 1].is_alphabetic()
            && chars.get(i + 1).is_some_and(|n| n.is_alphabetic())
        {
            abbreviation = Some("a letter, a dot and a letter (as in \"e.g.\")".to_owned());
            break;
        }
    }
    if abbreviation.is_none() {
        for word in text.split_whitespace() {
            let lowered = word
                .trim_matches(|c: char| !c.is_alphanumeric() && c != '.')
                .to_lowercase();
            let is_abbreviation = lowered
                .strip_suffix('.')
                .is_some_and(|stem| SPOKEN_ABBREVIATIONS.contains(&stem));
            if is_abbreviation {
                abbreviation = Some(format!("the abbreviation \"{word}\""));
                break;
            }
        }
    }
    if let Some(what) = abbreviation {
        problems.push(format!(
            "contains {what}; write the word out, because TTS may read it wrongly"
        ));
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_words_not_marks() {
        assert_eq!(word_count("Hello! My name is Dewi."), 5);
        assert_eq!(word_count("Class chat\n\nSari: Hi - there"), 5);
        assert_eq!(word_count("  ...  "), 0);
    }

    #[test]
    fn lowercase_words_keep_apostrophes_and_drop_gaps() {
        assert_eq!(
            lowercase_words("Dewi: \"What's your ___?\""),
            ["dewi", "what's", "your"]
        );
        assert_eq!(lowercase_words("I\u{2019}m fine."), ["i'm", "fine"]);
    }

    #[test]
    fn normalisation_equates_contractions_case_and_final_marks() {
        assert_eq!(
            normalise_answer("I'm Dewi."),
            normalise_answer("I am  Dewi")
        );
        assert_eq!(
            normalise_answer("Where are you from?"),
            "where are you from"
        );
        assert_ne!(normalise_answer("I Dewi."), normalise_answer("I am Dewi."));
        assert_eq!(
            normalise_answer("I\u{2019}m Dewi"),
            normalise_answer("i am dewi!")
        );
    }

    #[test]
    fn splits_sentences() {
        assert_eq!(
            sentences("Hello! I am Dewi. Where are you from?\nFine"),
            ["Hello", "I am Dewi", "Where are you from", "Fine"]
        );
    }

    #[test]
    fn similarity_is_one_for_equal_word_sets() {
        assert!((word_set_similarity("I am from Bali.", "i am from bali") - 1.0).abs() < 1e-9);
        assert!(word_set_similarity("I am from Bali", "She plays football") < 0.1);
        assert!(word_set_similarity("", "x") < 1e-9);
    }

    #[test]
    fn tts_text_rules() {
        assert!(tts_problems("Good evening! I'm Dewi. What's your name?").is_empty());
        assert!(tts_problems("Well-known, isn't it?").is_empty());
        assert_eq!(tts_problems("See you (soon)").len(), 1);
        assert_eq!(tts_problems("A/B test").len(), 1);
        assert_eq!(tts_problems("Line one\nline two").len(), 1);
        assert_eq!(tts_problems("Good day, e.g. now").len(), 1);
        assert_eq!(tts_problems("Hello, Mr. Hadi.").len(), 1);
        assert!(tts_problems("The price is 3.5 dollars.").is_empty());
        assert_eq!(tts_problems("It costs $5 e.g. now").len(), 2);
    }
}
