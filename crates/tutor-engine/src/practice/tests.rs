use std::path::Path;

use super::*;

fn example_unit() -> Unit {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../curriculum/examples/a1-u01.example.json");
    curriculum::load_unit_file(&path)
        .expect("the example unit loads")
        .unit
}

fn mcq() -> RawItem {
    RawItem {
        kind: GeneratedType::Mcq,
        objective_id: "o2-introduce".into(),
        grammar_id: "g-be-i-am".into(),
        stem: "I ___ Dewi.".into(),
        options: vec!["am".into(), "is".into(), "are".into()],
        answer_index: 0,
        text: String::new(),
        answers: vec![],
        tokens: vec![],
        answer: String::new(),
        explanation_en: "Use am with I.".into(),
        explanation_l1: "Pakai am dengan I.".into(),
    }
}

fn gap() -> RawItem {
    RawItem {
        kind: GeneratedType::GapFill,
        stem: String::new(),
        options: vec![],
        answer_index: -1,
        text: "My name ___ Budi.".into(),
        answers: vec![vec!["is".into()]],
        ..mcq()
    }
}

fn reorder() -> RawItem {
    RawItem {
        kind: GeneratedType::Reorder,
        stem: String::new(),
        options: vec![],
        answer_index: -1,
        tokens: vec!["from".into(), "I".into(), "am".into(), "Bali".into()],
        answer: "I am from Bali".into(),
        ..mcq()
    }
}

fn check(
    raw: &RawItem,
    unit: &Unit,
    existing: &[String],
    vocab: &Vocabulary,
) -> Result<Activity, Rejection> {
    check_item(raw, "gen-1".into(), unit, existing, vocab, true)
}

fn plain(unit: &Unit) -> Vocabulary {
    Vocabulary::for_unit(unit, None)
}

#[test]
fn a_valid_item_of_each_type_becomes_a_typed_generated_activity() {
    let unit = example_unit();
    let vocab = plain(&unit);
    for raw in [mcq(), gap(), reorder()] {
        let activity = check(&raw, &unit, &[], &vocab).expect("valid item");
        assert_eq!(activity.id(), "gen-1");
        assert_eq!(
            activity.common().objective_ids,
            std::slice::from_ref(&raw.objective_id)
        );
        assert_eq!(activity.common().scoring, Scoring::Deterministic);
    }
}

#[test]
fn explanations_carry_the_indonesian_text_only_for_indonesian_learners() {
    let unit = example_unit();
    let raw = mcq();
    let Activity::Mcq(with) = convert(&raw, "g".into(), true).expect("converts") else {
        panic!("not an mcq");
    };
    assert_eq!(with.explanation.id.as_deref(), Some("Pakai am dengan I."));
    let Activity::Mcq(without) = convert(&raw, "g".into(), false).expect("converts") else {
        panic!("not an mcq");
    };
    assert_eq!(without.explanation.id, None);
    let _ = unit;
}

#[test]
fn malformed_items_are_rejected_for_their_shape() {
    let unit = example_unit();
    let vocab = plain(&unit);
    let cases: Vec<(&str, RawItem)> = vec![
        (
            "two options",
            RawItem {
                options: vec!["am".into(), "is".into()],
                ..mcq()
            },
        ),
        (
            "five options",
            RawItem {
                options: ["a", "b", "c", "d", "e"].map(String::from).to_vec(),
                ..mcq()
            },
        ),
        (
            "same options",
            RawItem {
                options: vec!["am".into(), "AM".into(), "is".into()],
                ..mcq()
            },
        ),
        (
            "answer outside",
            RawItem {
                answer_index: 3,
                ..mcq()
            },
        ),
        (
            "negative answer",
            RawItem {
                answer_index: -1,
                ..mcq()
            },
        ),
        (
            "empty stem",
            RawItem {
                stem: " ".into(),
                ..mcq()
            },
        ),
        (
            "no explanation",
            RawItem {
                explanation_en: String::new(),
                ..mcq()
            },
        ),
        (
            "three gaps",
            RawItem {
                text: "___ name ___ Budi ___".into(),
                ..gap()
            },
        ),
        (
            "no gap",
            RawItem {
                text: "My name is Budi.".into(),
                ..gap()
            },
        ),
        (
            "empty accepted answer",
            RawItem {
                answers: vec![vec![]],
                ..gap()
            },
        ),
        (
            "two tokens",
            RawItem {
                tokens: vec!["I".into(), "am".into()],
                answer: "I am".into(),
                ..reorder()
            },
        ),
        (
            "final punctuation",
            RawItem {
                answer: "I am from Bali.".into(),
                ..reorder()
            },
        ),
    ];
    for (name, raw) in cases {
        assert!(
            matches!(check(&raw, &unit, &[], &vocab), Err(Rejection::BadShape(_))),
            "{name}"
        );
    }
}

#[test]
fn the_unit_validators_catch_what_the_shape_rules_do_not() {
    let unit = example_unit();
    let vocab = plain(&unit);
    // Two gaps but one answer list: E06.
    let two_gaps = RawItem {
        text: "I ___ Dewi and I ___ a student.".into(),
        ..gap()
    };
    assert_eq!(
        check(&two_gaps, &unit, &[], &vocab),
        Err(Rejection::Validator("E06".into()))
    );
    // Tokens already in answer order: E07.
    let in_order = RawItem {
        tokens: vec!["I".into(), "am".into(), "from".into(), "Bali".into()],
        ..reorder()
    };
    assert_eq!(
        check(&in_order, &unit, &[], &vocab),
        Err(Rejection::Validator("E07".into()))
    );
    // Tokens that do not make the answer: E07.
    let wrong_tokens = RawItem {
        tokens: vec!["from".into(), "I".into(), "was".into(), "Bali".into()],
        ..reorder()
    };
    assert_eq!(
        check(&wrong_tokens, &unit, &[], &vocab),
        Err(Rejection::Validator("E07".into()))
    );
}

#[test]
fn the_policy_and_the_unit_decide_which_items_are_allowed() {
    let mut unit = example_unit();
    let vocab = plain(&unit);
    assert_eq!(
        check(
            &RawItem {
                objective_id: "o-made-up".into(),
                ..mcq()
            },
            &unit,
            &[],
            &vocab
        ),
        Err(Rejection::UnknownObjective)
    );
    assert_eq!(
        check(
            &RawItem {
                grammar_id: "g-made-up".into(),
                ..mcq()
            },
            &unit,
            &[],
            &vocab
        ),
        Err(Rejection::UnknownGrammar)
    );
    unit.generation_policy.allowed_types = vec![GeneratedType::Mcq];
    assert_eq!(
        check(&gap(), &unit, &[], &vocab),
        Err(Rejection::TypeNotAllowed)
    );
    assert!(check(&mcq(), &unit, &[], &vocab).is_ok());
}

#[test]
fn a_repeat_or_light_rewording_of_an_existing_item_is_dropped() {
    let unit = example_unit();
    let vocab = plain(&unit);
    let existing = vec!["I ___ Dewi.".to_owned()];
    assert_eq!(
        check(&mcq(), &unit, &existing, &vocab),
        Err(Rejection::Repeat)
    );
    let reworded = RawItem {
        stem: "i ___ Dewi".into(),
        ..mcq()
    };
    assert_eq!(
        check(&reworded, &unit, &existing, &vocab),
        Err(Rejection::Repeat)
    );
    let different = RawItem {
        stem: "She ___ from Bali.".into(),
        ..mcq()
    };
    assert!(check(&different, &unit, &existing, &vocab).is_ok());
}

#[test]
fn the_vocabulary_check_uses_the_word_list_and_exempts_unit_words_and_names() {
    let unit = example_unit();
    let levels =
        WordLevels::parse("elephant,C1\nthe,A1\ni,A1\nam,A1\nname,A1\nzoo,C2\n").expect("list");
    let vocab = Vocabulary::for_unit(&unit, Some(Arc::new(levels)));
    assert!(vocab.is_checked());
    let hard = RawItem {
        stem: "The elephant is big.".into(),
        ..mcq()
    };
    assert_eq!(check(&hard, &unit, &[], &vocab), Err(Rejection::Vocabulary));
    // Names and words the list does not know are not flagged.
    let names = RawItem {
        stem: "I met Zoo Wang.".into(),
        ..mcq()
    };
    assert!(check(&names, &unit, &[], &vocab).is_ok());
    // A hard word in an option is found too.
    let option = RawItem {
        options: vec!["am".into(), "is".into(), "elephant".into()],
        ..mcq()
    };
    assert_eq!(
        check(&option, &unit, &[], &vocab),
        Err(Rejection::Vocabulary)
    );
}

#[test]
fn without_a_word_list_the_vocabulary_check_is_skipped_and_says_so() {
    let unit = example_unit();
    let vocab = plain(&unit);
    assert!(!vocab.is_checked());
    assert!(vocab.above_level("The elephant is enormous.").is_empty());
}

#[test]
fn content_words_skip_names_numbers_and_the_gap_marker() {
    assert_eq!(
        content_words("Dewi met Putu in Bali. She has 3 cats ___"),
        ["dewi", "met", "in", "she", "has", "cats"]
    );
}
