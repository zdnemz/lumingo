/// Version of the answer normaliser. It is stored with every deterministic
/// score, so a change here is a new version and old scores stay explainable.
pub const NORM_VERSION: &str = "norm/1";

/// Contractions and their full forms. Both sides normalise to the full form, so
/// "I'm" and "I am" are equal. The table is fixed on purpose: where a form is
/// ambiguous ("he's" can be "he is" or "he has") the first reading is used.
const CONTRACTIONS: [(&str, &str); 41] = [
    ("i'm", "i am"),
    ("you're", "you are"),
    ("he's", "he is"),
    ("she's", "she is"),
    ("it's", "it is"),
    ("we're", "we are"),
    ("they're", "they are"),
    ("that's", "that is"),
    ("what's", "what is"),
    ("there's", "there is"),
    ("here's", "here is"),
    ("who's", "who is"),
    ("where's", "where is"),
    ("i've", "i have"),
    ("you've", "you have"),
    ("we've", "we have"),
    ("they've", "they have"),
    ("i'll", "i will"),
    ("you'll", "you will"),
    ("he'll", "he will"),
    ("she'll", "she will"),
    ("we'll", "we will"),
    ("they'll", "they will"),
    ("i'd", "i would"),
    ("let's", "let us"),
    ("don't", "do not"),
    ("doesn't", "does not"),
    ("didn't", "did not"),
    ("can't", "can not"),
    ("cannot", "can not"),
    ("won't", "will not"),
    ("isn't", "is not"),
    ("aren't", "are not"),
    ("wasn't", "was not"),
    ("weren't", "were not"),
    ("haven't", "have not"),
    ("hasn't", "has not"),
    ("hadn't", "had not"),
    ("wouldn't", "would not"),
    ("shouldn't", "should not"),
    ("couldn't", "could not"),
];

fn expand_word(word: &str) -> Option<&'static str> {
    CONTRACTIONS
        .iter()
        .find(|(short, _)| *short == word)
        .map(|(_, full)| *full)
}

/// Normalises an answer for comparison, version `norm/1`: trim, lowercase,
/// collapse whitespace, turn curly quotes and apostrophes into straight ones,
/// drop final `.`, `?` and `!`, and write contractions out in full.
pub fn normalize(text: &str) -> String {
    let straightened: String = text
        .trim()
        .chars()
        .map(|c| match c {
            '\u{2018}' | '\u{2019}' | '\u{201b}' => '\'',
            '\u{201c}' | '\u{201d}' => '"',
            other => other,
        })
        .flat_map(char::to_lowercase)
        .collect();
    let trimmed = straightened.trim_end_matches(['.', '?', '!']).trim_end();
    let words: Vec<String> = trimmed
        .split_whitespace()
        .map(|token| {
            // Keep punctuation that touches a word ("i'm," or "(it's") while expanding the word.
            let start = token
                .find(|c: char| c.is_alphanumeric() || c == '\'')
                .unwrap_or(token.len());
            let end = token
                .rfind(|c: char| c.is_alphanumeric() || c == '\'')
                .map_or(start, |i| {
                    i + token[i..].chars().next().map_or(1, char::len_utf8)
                });
            let (lead, rest) = token.split_at(start);
            let (core, trail) = rest.split_at(end.saturating_sub(start).min(rest.len()));
            match expand_word(core) {
                Some(full) => format!("{lead}{full}{trail}"),
                None => token.to_owned(),
            }
        })
        .collect();
    words.join(" ")
}

/// Edit distance counting an insertion, a deletion, a substitution, or the swap
/// of two neighbouring characters as one edit (optimal string alignment).
pub fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (n, m) = (a.len(), b.len());
    let mut table = vec![vec![0_usize; m + 1]; n + 1];
    for (i, row) in table.iter_mut().enumerate() {
        row[0] = i;
    }
    table[0] = (0..=m).collect();
    for i in 1..=n {
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut best = (table[i - 1][j] + 1)
                .min(table[i][j - 1] + 1)
                .min(table[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(table[i - 2][j - 2] + 1);
            }
            table[i][j] = best;
        }
    }
    table[n][m]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_lowercases_and_collapses_whitespace() {
        assert_eq!(normalize("  Good   MORNING  "), "good morning");
        assert_eq!(normalize("a\tb\nc"), "a b c");
    }

    #[test]
    fn drops_only_the_final_sentence_marks() {
        assert_eq!(normalize("Hello there."), "hello there");
        assert_eq!(normalize("Really?!"), "really");
        assert_eq!(normalize("Hi, how are you?"), "hi, how are you");
        assert_eq!(normalize("3.5 is a number."), "3.5 is a number");
    }

    #[test]
    fn maps_curly_quotes_and_apostrophes_to_straight_ones() {
        assert_eq!(normalize("I\u{2019}m Dewi"), normalize("I'm Dewi"));
        assert_eq!(normalize("\u{201c}Hi\u{201d}"), "\"hi\"");
    }

    #[test]
    fn contractions_equal_their_full_forms() {
        assert_eq!(normalize("I'm Dewi"), normalize("I am Dewi"));
        assert_eq!(normalize("I don't know."), normalize("I do not know"));
        assert_eq!(normalize("She isn't here"), normalize("she is not here"));
        assert_eq!(normalize("We can't go"), normalize("We cannot go"));
        assert_eq!(normalize("It\u{2019}s fine"), normalize("it is fine"));
    }

    #[test]
    fn expands_a_contraction_that_touches_punctuation() {
        assert_eq!(normalize("Hello, I'm Dewi"), "hello, i am dewi");
        assert_eq!(normalize("(it's) fine"), "(it is) fine");
    }

    #[test]
    fn possessives_and_unlisted_forms_are_left_alone() {
        assert_eq!(normalize("Dewi's book"), "dewi's book");
        assert_ne!(normalize("her book"), normalize("his book"));
    }

    #[test]
    fn normalising_twice_changes_nothing() {
        for text in ["I'm here.", "  Don't  go! ", "Hello, I\u{2019}m Dewi?", ""] {
            let once = normalize(text);
            assert_eq!(normalize(&once), once, "{text:?}");
        }
    }

    #[test]
    fn edit_distance_counts_each_kind_of_edit_once() {
        assert_eq!(edit_distance("kitten", "kitten"), 0);
        assert_eq!(edit_distance("cat", "cut"), 1);
        assert_eq!(edit_distance("cat", "cats"), 1);
        assert_eq!(edit_distance("cats", "cat"), 1);
        assert_eq!(edit_distance("teh", "the"), 1);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("été", "ete"), 2);
    }

    #[test]
    fn the_version_is_stable() {
        assert_eq!(NORM_VERSION, "norm/1");
    }
}
