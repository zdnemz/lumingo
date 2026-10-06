//! Deterministic scoring and the `norm/1` normaliser (ASSESSMENT_SPEC section 4).
//! Every function takes plain answers and returns a score from 0 to 1. Confidence
//! of these attempts is always 1.0 and a success is a score of at least 0.7.

pub const NORMALISER_VERSION: &str = "norm/1";

/// Contractions and their full forms. Both sides normalise to the full form, so
/// "I'm" equals "I am". `'s` is read as "is" (never as "has" or a possessive), so
/// "he's" matches "he is"; possessives like "Sari's" are left alone. `'d` is
/// ambiguous (would or had) and is not in the table.
const CONTRACTIONS: [(&str, &str); 36] = [
    ("i'm", "i am"),
    ("you're", "you are"),
    ("we're", "we are"),
    ("they're", "they are"),
    ("he's", "he is"),
    ("she's", "she is"),
    ("it's", "it is"),
    ("that's", "that is"),
    ("there's", "there is"),
    ("what's", "what is"),
    ("here's", "here is"),
    ("isn't", "is not"),
    ("aren't", "are not"),
    ("wasn't", "was not"),
    ("weren't", "were not"),
    ("don't", "do not"),
    ("doesn't", "does not"),
    ("didn't", "did not"),
    ("can't", "cannot"),
    ("won't", "will not"),
    ("wouldn't", "would not"),
    ("shouldn't", "should not"),
    ("couldn't", "could not"),
    ("haven't", "have not"),
    ("hasn't", "has not"),
    ("hadn't", "had not"),
    ("i've", "i have"),
    ("you've", "you have"),
    ("we've", "we have"),
    ("they've", "they have"),
    ("i'll", "i will"),
    ("you'll", "you will"),
    ("he'll", "he will"),
    ("she'll", "she will"),
    ("we'll", "we will"),
    ("let's", "let us"),
];

/// `norm/1`: trim, lowercase, collapse whitespace, straighten curly quotes and
/// apostrophes, drop final `.`, `?` and `!`, and expand contractions.
pub fn normalize(text: &str) -> String {
    let straight: String = text
        .chars()
        .map(|c| match c {
            '\u{2018}' | '\u{2019}' | '\u{201B}' => '\'',
            '\u{201C}' | '\u{201D}' => '"',
            c => c,
        })
        .collect();
    let lowered = straight.to_lowercase();
    let trimmed =
        lowered.trim_end_matches(|c: char| c.is_whitespace() || matches!(c, '.' | '?' | '!'));
    let mut words: Vec<&str> = Vec::new();
    for word in trimmed.split_whitespace() {
        match CONTRACTIONS.iter().find(|(short, _)| *short == word) {
            Some((_, full)) => words.extend(full.split(' ')),
            None => words.push(word),
        }
    }
    words.join(" ")
}

/// `can not` and `cannot` are the same answer.
fn canonical(text: &str) -> String {
    normalize(text).replace("can not", "cannot")
}

/// 1 for a match with any accepted answer after normalisation, else 0. Serves
/// `reorder`, `error_correction` and exact-answer items.
pub fn score_exact(given: &str, accepted: &[&str]) -> f64 {
    let given = canonical(given);
    f64::from(accepted.iter().any(|a| canonical(a) == given))
}

/// `mcq` and each question of a set: 1 or 0.
pub fn score_choice(given: usize, correct: usize) -> f64 {
    f64::from(given == correct)
}

/// Share of items correct, for `match`, `minimal_pairs`, `reading_set` and
/// `listening_set`. No items scores 0 so an empty activity is never a success.
pub fn score_share(correct: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        correct as f64 / total as f64
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GapScore {
    pub score: f64,
    /// Indexes of gaps that earned half credit for a one-letter slip.
    pub spelling_notes: Vec<usize>,
}

/// Share of gaps correct. An answer one edit away from an accepted answer of
/// five letters or more earns 0.5 and a spelling note. `accepted[i]` lists the
/// accepted answers for gap `i`; a missing answer scores 0.
pub fn score_gap_fill(given: &[&str], accepted: &[Vec<&str>]) -> GapScore {
    let mut total = 0.0;
    let mut spelling_notes = Vec::new();
    for (i, options) in accepted.iter().enumerate() {
        let Some(answer) = given.get(i).map(|g| canonical(g)) else {
            continue;
        };
        if options.iter().any(|o| canonical(o) == answer) {
            total += 1.0;
        } else if options.iter().any(|o| {
            let o = canonical(o);
            o.chars().count() >= 5
                && edit_distance(
                    &o.chars().collect::<Vec<_>>(),
                    &answer.chars().collect::<Vec<_>>(),
                ) == 1
        }) {
            total += 0.5;
            spelling_notes.push(i);
        }
    }
    GapScore {
        score: score_share_f(total, accepted.len()),
        spelling_notes,
    }
}

fn score_share_f(points: f64, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        points / total as f64
    }
}

/// 1 minus the word error rate against the best accepted answer, not below 0.
pub fn score_dictation(given: &str, accepted: &[&str]) -> f64 {
    let hyp: Vec<String> = canonical(given)
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    accepted
        .iter()
        .map(|a| {
            let reference: Vec<String> =
                canonical(a).split_whitespace().map(str::to_owned).collect();
            if reference.is_empty() {
                return f64::from(hyp.is_empty());
            }
            (1.0 - edit_distance(&reference, &hyp) as f64 / reference.len() as f64).max(0.0)
        })
        .fold(0.0, f64::max)
}

/// Levenshtein distance over any comparable items (characters or words).
fn edit_distance<T: PartialEq>(a: &[T], b: &[T]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, x) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, y) in b.iter().enumerate() {
            cur.push(
                (prev[j] + usize::from(x != y))
                    .min(prev[j + 1] + 1)
                    .min(cur[j] + 1),
            );
        }
        prev = cur;
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalisation_follows_norm_1() {
        assert_eq!(normalize("  Hello \t  World?! "), "hello world");
        assert_eq!(
            normalize("It\u{2019}s \u{201C}fine\u{201D}."),
            "it is \"fine\""
        );
        assert_eq!(normalize("She said no..."), "she said no");
        assert_eq!(normalize("Sari's book."), "sari's book");
    }

    #[test]
    fn contractions_equal_their_full_forms() {
        for (short, full) in [
            ("I'm happy.", "I am happy"),
            ("We don\u{2019}t know", "we do not know"),
            ("He's here", "he is here"),
            ("Let's go!", "Let us go"),
        ] {
            assert_eq!(score_exact(short, &[full]), 1.0, "{short}");
            assert_eq!(score_exact(full, &[short]), 1.0, "{full}");
        }
        assert_eq!(score_exact("I can't go", &["I can not go"]), 1.0);
    }

    #[test]
    fn exact_items_accept_any_listed_answer_and_nothing_else() {
        assert_eq!(
            score_exact(
                "She goes to school.",
                &["She goes to school", "She is going to school"]
            ),
            1.0
        );
        assert_eq!(
            score_exact("She go to school", &["She goes to school"]),
            0.0
        );
        assert_eq!(score_exact("", &["x"]), 0.0);
    }

    #[test]
    fn choice_and_share_scores() {
        assert_eq!((score_choice(2, 2), score_choice(1, 2)), (1.0, 0.0));
        assert_eq!(score_share(3, 4), 0.75);
        assert_eq!(score_share(0, 0), 0.0);
    }

    #[test]
    fn gap_fill_gives_half_credit_for_one_edit_only_on_long_answers() {
        let accepted = vec![vec!["morning"], vec!["went", "go"], vec!["tomorrow"]];
        // exact, exact via the other accepted answer, one edit away from a long word
        let s = score_gap_fill(&["morning", "go", "tomorow"], &accepted);
        assert_eq!(s.spelling_notes, [2]);
        assert!((s.score - (2.5 / 3.0)).abs() < 1e-9);
        // "wnt" is one edit from "went", but "went" has four letters: no credit
        assert_eq!(
            score_gap_fill(&["morning", "wnt", "tomorrow"], &accepted).score,
            2.0 / 3.0
        );
        // two edits away earns nothing; a missing answer earns nothing
        assert_eq!(score_gap_fill(&["mornxxg"], &accepted).score, 0.0);
    }

    #[test]
    fn dictation_is_one_minus_word_error_rate_against_the_best_answer() {
        assert_eq!(score_dictation("I like tea", &["I like tea."]), 1.0);
        assert!((score_dictation("I like", &["I like tea"]) - 2.0 / 3.0).abs() < 1e-9);
        // one extra word against the closer answer: word error rate 1/3
        let best = score_dictation("I love hot tea", &["I like tea", "I love tea"]);
        assert!((best - 2.0 / 3.0).abs() < 1e-9);
        assert_eq!(score_dictation("a b c d e f", &["x y"]), 0.0);
        assert_eq!(score_dictation("", &["I like tea"]), 0.0);
    }
}
