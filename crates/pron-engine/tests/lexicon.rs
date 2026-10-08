#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
//! S3-08 verification: lookup on 50 or more words, including variants and
//! words outside the lexicon. The lexicon is hand-written test data, not CMUdict.

use pron_engine::{Arpabet, Lexicon, NotChecked, Reference, WordStatus};

const TEST_LEXICON: &str = include_str!("data/test_lexicon.dict");

fn lexicon() -> Lexicon {
    Lexicon::parse(TEST_LEXICON).expect("the test lexicon parses")
}

fn spelled(variant: &[pron_engine::Phone]) -> String {
    variant
        .iter()
        .map(|p| p.symbol.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn the_test_lexicon_says_it_is_not_cmudict() {
    let first_lines: String = TEST_LEXICON.lines().take(3).collect::<Vec<_>>().join("\n");
    assert!(first_lines.contains("NOT CMUdict"));
}

#[test]
fn fifty_words_with_expected_pronunciations() {
    // (word, expected pronunciations without stress digits, in file order)
    let table: [(&str, &[&str]); 52] = [
        ("a", &["AH", "EY"]),
        ("asked", &["AE S K T"]),
        ("about", &["AH B AW T"]),
        ("bad", &["B AE D"]),
        ("bag", &["B AE G"]),
        ("bed", &["B EH D"]),
        ("can", &["K AE N", "K AH N"]),
        ("cat", &["K AE T"]),
        ("coffee", &["K AO F IY", "K AA F IY"]),
        ("data", &["D EY T AH", "D AE T AH", "D AA T AH"]),
        ("do", &["D UW"]),
        ("dog", &["D AO G"]),
        ("don't", &["D OW N T"]),
        ("easy", &["IY Z IY"]),
        ("either", &["IY DH ER", "AY DH ER"]),
        ("fish", &["F IH SH"]),
        ("fool", &["F UW L"]),
        ("from", &["F R AH M", "F R AH M"]),
        ("full", &["F UH L"]),
        ("good", &["G UH D"]),
        ("hello", &["HH AH L OW", "HH EH L OW"]),
        ("help", &["HH EH L P"]),
        ("i'm", &["AY M"]),
        ("job", &["JH AA B"]),
        ("live", &["L IH V", "L AY V"]),
        ("morning", &["M AO R N IH NG"]),
        ("mother", &["M AH DH ER"]),
        ("name", &["N EY M"]),
        ("please", &["P L IY Z"]),
        ("read", &["R EH D", "R IY D"]),
        ("record", &["R EH K ER D", "R IH K AO R D"]),
        ("school", &["S K UW L"]),
        ("seven", &["S EH V AH N"]),
        ("she", &["SH IY"]),
        ("sheep", &["SH IY P"]),
        ("ship", &["SH IH P"]),
        ("texts", &["T EH K S T S"]),
        ("thank", &["TH AE NG K"]),
        ("this", &["DH IH S"]),
        ("three", &["TH R IY"]),
        ("the", &["DH AH", "DH IY"]),
        ("to", &["T UW", "T AH"]),
        ("today", &["T AH D EY"]),
        ("tomato", &["T AH M EY T OW", "T AH M AA T OW"]),
        ("very", &["V EH R IY"]),
        ("vision", &["V IH ZH AH N"]),
        ("water", &["W AO T ER", "W AA T ER"]),
        ("what", &["W AH T", "W AH T"]),
        ("world", &["W ER L D"]),
        ("yes", &["Y EH S"]),
        ("you", &["Y UW", "Y UH", "Y AH"]),
        ("zoo", &["Z UW"]),
    ];
    let lex = lexicon();
    for (word, expected) in table {
        let found = lex
            .lookup(word)
            .unwrap_or_else(|| panic!("`{word}` should be in the test lexicon"));
        let got: Vec<String> = found.iter().map(|v| spelled(v)).collect();
        // "from" and "what" differ only in stress, which `spelled` hides, so
        // the table lists the symbol string twice for them.
        assert_eq!(got, expected, "pronunciations of `{word}`");
    }
}

#[test]
fn words_outside_the_lexicon_are_reported_not_guessed() {
    let lex = lexicon();
    for word in ["ayam", "jakarta", "xylophone", "zzz", "thx"] {
        assert!(lex.lookup(word).is_none(), "{word}");
    }
    let r = Reference::from_text("Thank you, Jakarta! Call 911.", &lex);
    let skipped: Vec<_> = r.not_checked();
    assert_eq!(
        skipped,
        [
            ("jakarta", NotChecked::NotInLexicon),
            ("call", NotChecked::NotInLexicon),
            ("911", NotChecked::Number),
        ]
    );
    assert_eq!(r.checked_count(), 2);
}

#[test]
fn a_sentence_resolves_every_variant() {
    let lex = lexicon();
    let r = Reference::from_text("Hello, I'm from the world.", &lex);
    let counts: Vec<usize> = r
        .words
        .iter()
        .map(|w| match &w.status {
            WordStatus::Checked { pronunciations } => pronunciations.len(),
            WordStatus::NotChecked { .. } => 0,
        })
        .collect();
    assert_eq!(counts, [2, 1, 2, 2, 1]);
}

#[test]
fn the_lexicon_only_uses_known_symbols() {
    let used = lexicon().symbols_in_use();
    assert!(used.contains(&Arpabet::ZH));
    assert!(used.contains(&Arpabet::NG));
    assert!(used.len() > 25);
}

/// Run with `LUMINGO_CMUDICT=/path/to/cmudict.dict cargo test -p pron-engine --
/// --ignored`. It checks that the loader reads a real dictionary completely and
/// that every symbol the dictionary uses is one of the 39 the engine knows.
#[test]
#[ignore = "needs a user-supplied cmudict.dict: set LUMINGO_CMUDICT"]
fn a_user_supplied_dictionary_loads_completely() {
    let path =
        std::env::var("LUMINGO_CMUDICT").expect("set LUMINGO_CMUDICT to a cmudict.dict path");
    let lex = Lexicon::from_path(std::path::Path::new(&path)).expect("the dictionary loads");
    assert!(lex.len() > 100_000, "only {} words", lex.len());
    assert_eq!(lex.symbols_in_use().len(), 39);
    let hello = lex.lookup("hello").expect("hello is in a real dictionary");
    assert!(!hello.is_empty());
}
