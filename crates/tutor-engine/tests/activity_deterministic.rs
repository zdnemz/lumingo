#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! The deterministic activities of the example unit: presentation, scoring,
//! feedback data and the rules of the spec (contractions, the one-edit rule,
//! replays). Persistence is tested with the unit player.

mod common;

use common::example_unit;
use curriculum::{Activity, Localized, Scoring, Skill, Unit};
use serde_json::Value;
use tutor_engine::{
    ActivityError, Body, ItemOutcome, PlayCounter, Response, audio_of, evidence_skill,
    match_display_order, minimal_pair_spoken, present, score_deterministic,
};

fn activity<'a>(unit: &'a Unit, id: &str) -> &'a Activity {
    unit.activities
        .iter()
        .find(|a| a.id() == id)
        .unwrap_or_else(|| panic!("the example unit has {id}"))
}

fn picks(items: &[Option<usize>]) -> Response {
    Response::Picks(items.to_vec())
}

fn text(s: &str) -> Response {
    Response::Text(s.to_owned())
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

fn localized(en: &str) -> Localized {
    Localized {
        en: en.to_owned(),
        id: None,
    }
}

fn score(unit: &Unit, id: &str, response: &Response) -> tutor_engine::Scored {
    score_deterministic(activity(unit, id), response).expect("scored")
}

#[test]
fn mcq_is_one_or_zero_and_the_feedback_names_the_right_option() {
    let unit = example_unit();
    let right = score(&unit, "a02-greeting-by-time", &Response::Choice(1));
    assert_eq!(right.record.normalized, 1.0);
    assert_eq!(right.feedback.items[0].outcome, ItemOutcome::Correct);
    assert!(right.feedback.passed);
    assert_eq!(right.record.algorithm, "mcq/1");
    let wrong = score(&unit, "a02-greeting-by-time", &Response::Choice(2));
    assert_eq!(wrong.record.normalized, 0.0);
    assert_eq!(
        wrong.feedback.items[0].given.as_deref(),
        Some("Good night.")
    );
    assert_eq!(wrong.feedback.items[0].expected, "Good evening.");
    assert!(wrong.feedback.explanation.is_some());
    assert!(!wrong.feedback.passed);
}

#[test]
fn a_choice_outside_the_options_and_a_response_of_the_wrong_shape_are_refused() {
    let unit = example_unit();
    let mcq = activity(&unit, "a02-greeting-by-time");
    assert_eq!(
        score_deterministic(mcq, &Response::Choice(3)).unwrap_err(),
        ActivityError::OutOfRange {
            index: 3,
            options: 3
        }
    );
    assert!(matches!(
        score_deterministic(mcq, &text("Good evening.")),
        Err(ActivityError::WrongKind { .. })
    ));
    let set = activity(&unit, "a14-read-set-class-chat");
    assert_eq!(
        score_deterministic(set, &picks(&[Some(1), Some(2)])).unwrap_err(),
        ActivityError::WrongLength {
            expected: 4,
            got: 2
        }
    );
    assert!(matches!(
        score_deterministic(set, &picks(&[Some(1), Some(2), Some(2), Some(9)])),
        Err(ActivityError::OutOfRange { index: 9, .. })
    ));
}

#[test]
fn gap_fill_scores_the_share_of_gaps_and_names_each_outcome() {
    let unit = example_unit();
    let both = score(
        &unit,
        "a03-gap-am",
        &Response::Gaps(strings(&["AM", "from."])),
    );
    assert_eq!(both.record.normalized, 1.0);
    let one = score(
        &unit,
        "a03-gap-am",
        &Response::Gaps(strings(&["are", "from"])),
    );
    assert_eq!(one.record.normalized, 0.5);
    assert_eq!(one.record.raw, 1.0);
    assert_eq!(one.record.max, 2.0);
    let outcomes: Vec<ItemOutcome> = one.feedback.items.iter().map(|i| i.outcome).collect();
    assert_eq!(outcomes, [ItemOutcome::Wrong, ItemOutcome::Correct]);
    let blank = score(&unit, "a03-gap-am", &Response::Gaps(strings(&["", " "])));
    assert_eq!(blank.record.normalized, 0.0);
    assert!(
        blank
            .feedback
            .items
            .iter()
            .all(|i| i.outcome == ItemOutcome::Blank)
    );
    assert!(matches!(
        score_deterministic(
            activity(&unit, "a03-gap-am"),
            &Response::Gaps(strings(&["am"]))
        ),
        Err(ActivityError::WrongLength {
            expected: 2,
            got: 1
        })
    ));
}

fn synthetic_gap(answers: &[&[&str]]) -> Activity {
    Activity::GapFill(curriculum::GapFill {
        id: "g".into(),
        skill: Skill::Grammar,
        objective_ids: vec!["o".into()],
        instructions: localized("Fill the gap."),
        scoring: Scoring::Deterministic,
        text: answers.iter().map(|_| "___").collect::<Vec<_>>().join(" "),
        answers: answers.iter().map(|a| strings(a)).collect(),
        explanation: localized("x"),
    })
}

#[test]
fn gap_fill_gives_half_credit_and_a_spelling_note_for_one_edit_in_a_long_word_only() {
    let long = synthetic_gap(&[&["beautiful"]]);
    let near = score_deterministic(&long, &Response::Gaps(strings(&["beautifull"]))).unwrap();
    assert_eq!(near.record.normalized, 0.5);
    assert_eq!(near.feedback.items[0].outcome, ItemOutcome::Spelling);
    assert_eq!(near.record.notes.len(), 1);
    assert!(near.record.notes[0].contains("beautiful"));
    let far = score_deterministic(&long, &Response::Gaps(strings(&["beutifull"]))).unwrap();
    assert_eq!(far.record.normalized, 0.0);
    let short = synthetic_gap(&[&["tree"]]);
    let slip = score_deterministic(&short, &Response::Gaps(strings(&["tre"]))).unwrap();
    assert_eq!(
        slip.record.normalized, 0.0,
        "four letters earn no half credit"
    );
}

#[test]
fn reorder_needs_the_order_and_ignores_case() {
    let unit = example_unit();
    let right = score(
        &unit,
        "a04-reorder-name",
        &Response::Order(strings(&["my", "NAME", "is", "dewi"])),
    );
    assert_eq!(right.record.normalized, 1.0);
    let wrong = score(
        &unit,
        "a04-reorder-name",
        &Response::Order(strings(&["name", "My", "Dewi", "is"])),
    );
    assert_eq!(wrong.record.normalized, 0.0);
    assert_eq!(wrong.feedback.items[0].expected, "My name is Dewi");
    assert!(matches!(
        score_deterministic(
            activity(&unit, "a04-reorder-name"),
            &Response::Order(strings(&["My", "name"]))
        ),
        Err(ActivityError::WrongLength { .. })
    ));
}

/// The response that matches every left-hand phrase with its own meaning, in
/// the positions the presentation shows them.
fn perfect_matches(unit: &Unit, id: &str) -> Vec<Option<usize>> {
    let Activity::Match(a) = activity(unit, id) else {
        panic!("a match");
    };
    let order = match_display_order(id, a.pairs.len());
    (0..a.pairs.len())
        .map(|left| order.iter().position(|pair| *pair == left))
        .collect()
}

#[test]
fn match_scores_the_share_of_pairs_in_the_displayed_order() {
    let unit = example_unit();
    let id = "a05-match-phrases";
    let all = perfect_matches(&unit, id);
    assert_eq!(score(&unit, id, &picks(&all)).record.normalized, 1.0);
    // The right column is not in the authored order, so lining items up in the
    // order they appear is not the answer.
    let lined_up: Vec<Option<usize>> = (0..4).map(Some).collect();
    assert!(score(&unit, id, &picks(&lined_up)).record.normalized < 1.0);

    let mut three = all.clone();
    three[3] = None;
    let result = score(&unit, id, &picks(&three));
    assert_eq!(result.record.normalized, 0.75);
    assert_eq!(result.feedback.items[3].outcome, ItemOutcome::Blank);
    assert_eq!(
        result.feedback.items[3].expected,
        "Senang bertemu dengan Anda"
    );

    let mut swapped = all;
    swapped.swap(0, 1);
    let result = score(&unit, id, &picks(&swapped));
    assert_eq!(result.record.normalized, 0.5);
    assert_eq!(result.feedback.items[0].outcome, ItemOutcome::Wrong);
    assert_eq!(
        result.feedback.items[0].given.as_deref(),
        Some("Terima kasih")
    );
}

#[test]
fn the_match_presentation_shows_the_right_column_in_the_scoring_order() {
    let unit = example_unit();
    let shown = present(&unit, activity(&unit, "a05-match-phrases")).unwrap();
    let Body::Match { left, right } = shown.body else {
        panic!("a match body");
    };
    assert_eq!(left.len(), 4);
    let mut sorted = right.clone();
    sorted.sort();
    let mut authored = vec![
        "Selamat pagi",
        "Terima kasih",
        "Sampai jumpa",
        "Senang bertemu dengan Anda",
    ];
    authored.sort_unstable();
    assert_eq!(sorted, authored, "the same four meanings, reordered");
    assert_ne!(right[0], "Selamat pagi");
}

#[test]
fn dictation_is_one_minus_the_word_error_rate_and_contractions_match() {
    let unit = example_unit();
    assert_eq!(
        score(&unit, "a06-dictation-from", &text("where are you from"))
            .record
            .normalized,
        1.0
    );
    assert_eq!(
        score(&unit, "a06-dictation-from", &text("Where are you form?"))
            .record
            .normalized,
        0.75
    );
    let blank = score(&unit, "a06-dictation-from", &text(""));
    assert_eq!(blank.record.normalized, 0.0);
    assert_eq!(blank.feedback.items[0].outcome, ItemOutcome::Blank);

    let contraction = Activity::Dictation(curriculum::Dictation {
        id: "d".into(),
        skill: Skill::Listening,
        objective_ids: vec!["o".into()],
        instructions: localized("Type it."),
        scoring: Scoring::Deterministic,
        audio_text: "I'm Dewi and I don't know.".into(),
        accepted_answers: strings(&["I'm Dewi and I don't know."]),
    });
    let spelled_out =
        score_deterministic(&contraction, &text("I am Dewi and I do not know")).unwrap();
    assert_eq!(
        spelled_out.record.normalized, 1.0,
        "I'm equals I am, don't equals do not"
    );
}

#[test]
fn error_correction_needs_an_accepted_sentence_after_normalising() {
    let unit = example_unit();
    for right in ["I am Dewi.", "i'm dewi", "I'M DEWI!"] {
        assert_eq!(
            score(&unit, "a16-fix-missing-am", &text(right))
                .record
                .normalized,
            1.0,
            "{right}"
        );
    }
    let wrong = score(&unit, "a16-fix-missing-am", &text("I Dewi"));
    assert_eq!(wrong.record.normalized, 0.0);
    assert_eq!(wrong.feedback.items[0].expected, "I am Dewi.");
    assert!(wrong.feedback.explanation.is_some());
}

#[test]
fn reading_and_listening_sets_score_the_share_of_questions() {
    let unit = example_unit();
    let reading = "a14-read-set-class-chat";
    let all = score(
        &unit,
        reading,
        &picks(&[Some(1), Some(2), Some(2), Some(0)]),
    );
    assert_eq!(all.record.normalized, 1.0);
    assert_eq!(all.record.algorithm, "reading_set/1");
    let three = score(
        &unit,
        reading,
        &picks(&[Some(1), Some(2), Some(2), Some(1)]),
    );
    assert_eq!(three.record.normalized, 0.75);
    assert_eq!(three.feedback.correct, 3);
    assert_eq!(three.feedback.total, 4);
    assert!(three.feedback.items[3].explanation.is_some());
    let blank = score(&unit, reading, &picks(&[None, None, None, None]));
    assert_eq!(blank.record.normalized, 0.0);
    assert_eq!(blank.record.raw, 0.0);

    let listening = score(
        &unit,
        "a15-listen-set-putu",
        &picks(&[Some(0), Some(1), Some(0)]),
    );
    assert!((listening.record.normalized - 2.0 / 3.0).abs() < 1e-9);
    assert_eq!(listening.record.algorithm, "listening_set/1");
}

#[test]
fn listening_minimal_pairs_play_one_word_of_each_pair_and_score_the_share_right() {
    let unit = example_unit();
    let id = "a08-pairs-th";
    let Activity::MinimalPairs(a) = activity(&unit, id) else {
        panic!("minimal pairs");
    };
    let spoken: Vec<Option<usize>> = (0..a.pairs.len())
        .map(|i| Some(minimal_pair_spoken(id, i)))
        .collect();
    let perfect = score(&unit, id, &picks(&spoken));
    assert_eq!(perfect.record.normalized, 1.0);
    assert_eq!(perfect.record.algorithm, "minimal_pairs_listen/1");
    let flipped: Vec<Option<usize>> = spoken.iter().map(|s| s.map(|i| 1 - i)).collect();
    assert_eq!(score(&unit, id, &picks(&flipped)).record.normalized, 0.0);
    let mut one_wrong = spoken.clone();
    one_wrong[0] = one_wrong[0].map(|i| 1 - i);
    let two_of_three = score(&unit, id, &picks(&one_wrong));
    assert!((two_of_three.record.normalized - 2.0 / 3.0).abs() < 1e-9);
    assert!(matches!(
        score_deterministic(activity(&unit, id), &picks(&[Some(2), None, None])),
        Err(ActivityError::OutOfRange {
            index: 2,
            options: 2
        })
    ));
    // The audio holds the word the scoring expects.
    let lines = audio_of(&unit, activity(&unit, id)).unwrap();
    for (i, line) in lines.iter().enumerate() {
        let expected = if minimal_pair_spoken(id, i) == 0 {
            &a.pairs[i].a
        } else {
            &a.pairs[i].b
        };
        assert_eq!(&line.text, expected);
    }
}

#[test]
fn a_productive_activity_is_not_scored_here_and_the_error_names_its_scorer() {
    let unit = example_unit();
    let error =
        score_deterministic(activity(&unit, "a10-speak-introduce"), &text("Hello")).unwrap_err();
    assert_eq!(error, ActivityError::NotDeterministic("the rubric scorer"));
    let error =
        score_deterministic(activity(&unit, "a07-read-aloud-thanks"), &Response::Done).unwrap_err();
    assert_eq!(
        error,
        ActivityError::NotDeterministic("the pronunciation engine")
    );
}

#[test]
fn text_beyond_the_cap_is_refused_and_not_cut() {
    let unit = example_unit();
    let long = "a".repeat(tutor_engine::MAX_TEXT_CHARS + 1);
    assert!(matches!(
        score_deterministic(activity(&unit, "a06-dictation-from"), &text(&long)),
        Err(ActivityError::TooLong { .. })
    ));
    let at_cap = "a".repeat(tutor_engine::MAX_TEXT_CHARS);
    assert!(score_deterministic(activity(&unit, "a06-dictation-from"), &text(&at_cap)).is_ok());
}

#[test]
fn the_skill_of_every_activity_follows_section_two_of_the_spec() {
    let unit = example_unit();
    let expected = [
        ("a01-listen-question", "listening"),
        ("a02-greeting-by-time", "vocabulary"),
        ("a03-gap-am", "grammar"),
        ("a04-reorder-name", "grammar"),
        ("a05-match-phrases", "vocabulary"),
        ("a06-dictation-from", "listening"),
        ("a07-read-aloud-thanks", "pronunciation"),
        ("a08-pairs-th", "listening"),
        ("a09-shadow-dialogue", "pronunciation"),
        ("a10-speak-introduce", "speaking"),
        ("a11-roleplay-classmate", "speaking"),
        ("a12-write-introduce", "writing"),
        ("a13-read-budi", "reading"),
        ("a14-read-set-class-chat", "reading"),
        ("a15-listen-set-putu", "listening"),
        ("a16-fix-missing-am", "writing"),
        ("a17-fix-missing-from", "writing"),
    ];
    for (id, skill) in expected {
        assert_eq!(evidence_skill(activity(&unit, id), false), skill, "{id}");
    }
    let mediation = curriculum::Mediation {
        id: "m".into(),
        skill: Skill::Mediation,
        objective_ids: vec!["o".into()],
        instructions: localized("x"),
        scoring: Scoring::Rubric,
        source_text: "x".into(),
        task: localized("x"),
        rubric_id: "rubric-b1-mediation".into(),
        model_answers: Vec::new(),
    };
    let activity = Activity::Mediation(mediation);
    assert_eq!(evidence_skill(&activity, true), "speaking");
    assert_eq!(evidence_skill(&activity, false), "writing");
}

#[test]
fn a_presentation_never_carries_an_answer_a_model_answer_or_a_rubric() {
    let unit = example_unit();
    let forbidden = [
        "answer_index",
        "answers",
        "accepted_answers",
        "answer",
        "explanation",
        "model_answers",
        "rubric_id",
        "scoring",
    ];
    fn walk(value: &Value, forbidden: &[&str], path: &str) {
        match value {
            Value::Object(map) => {
                for (key, inner) in map {
                    assert!(
                        !forbidden.contains(&key.as_str()),
                        "{path}/{key} leaks an answer"
                    );
                    walk(inner, forbidden, &format!("{path}/{key}"));
                }
            }
            Value::Array(items) => {
                for (i, inner) in items.iter().enumerate() {
                    walk(inner, forbidden, &format!("{path}/{i}"));
                }
            }
            _ => {}
        }
    }
    for activity in &unit.activities {
        let shown = present(&unit, activity).expect("presents");
        assert_eq!(shown.id, activity.id());
        walk(
            &serde_json::to_value(&shown).unwrap(),
            &forbidden,
            activity.id(),
        );
    }
}

#[test]
fn the_audio_of_an_item_is_separate_from_what_is_displayed() {
    let unit = example_unit();
    let shown = present(&unit, activity(&unit, "a15-listen-set-putu")).unwrap();
    let Body::ListeningSet {
        audio,
        replays_allowed,
        questions,
    } = shown.body
    else {
        panic!("a listening set");
    };
    assert_eq!(replays_allowed, 2);
    assert_eq!(audio.len(), 1);
    assert!(audio[0].text.starts_with("Hello! My name is Putu."));
    assert_eq!(questions.len(), 3);
    assert!(
        !serde_json::to_string(&questions)
            .unwrap()
            .contains("Putu. I am from")
    );

    let shadow = present(&unit, activity(&unit, "a09-shadow-dialogue")).unwrap();
    let Body::Shadowing { title, lines } = shadow.body else {
        panic!("shadowing");
    };
    assert_eq!(title, "New classmates");
    assert!(lines.len() >= 4);
    assert_eq!(lines[0].speaker.as_deref(), Some("Dewi"));
}

#[test]
fn listening_sets_honour_their_replay_limit() {
    let unit = example_unit();
    let mut plays = PlayCounter::for_activity(activity(&unit, "a15-listen-set-putu"));
    assert_eq!(plays.play(), Ok(1));
    assert_eq!(plays.play(), Ok(2));
    assert_eq!(plays.play(), Ok(3));
    assert_eq!(
        plays.play(),
        Err(ActivityError::NoReplaysLeft { allowed: 2 })
    );
    // Other audio items have no limit in the content format.
    let mut free = PlayCounter::for_activity(activity(&unit, "a06-dictation-from"));
    for _ in 0..10 {
        assert!(free.play().is_ok());
    }
}

#[test]
fn feedback_data_never_names_a_level_or_states_a_percentage() {
    let unit = example_unit();
    let responses: Vec<(&str, Response)> = vec![
        ("a01-listen-question", Response::Choice(1)),
        ("a02-greeting-by-time", Response::Choice(0)),
        ("a03-gap-am", Response::Gaps(strings(&["is", "frm"]))),
        (
            "a04-reorder-name",
            Response::Order(strings(&["name", "My", "Dewi", "is"])),
        ),
        ("a06-dictation-from", text("where you from")),
        (
            "a14-read-set-class-chat",
            picks(&[Some(0), Some(0), Some(0), Some(2)]),
        ),
        ("a15-listen-set-putu", picks(&[Some(1), Some(0), Some(2)])),
        ("a16-fix-missing-am", text("Dewi am I")),
    ];
    for (id, response) in responses {
        let scored = score(&unit, id, &response);
        let json = serde_json::to_string(&scored.feedback).unwrap();
        assert!(!json.contains('%'), "{id}");
        let lower = json.to_lowercase();
        assert!(!lower.contains("cefr"), "{id}");
        for token in json.split(|c: char| !c.is_ascii_alphanumeric()) {
            assert!(
                !matches!(token, "A1" | "A2" | "B1" | "B2" | "C1" | "C2"),
                "{id} names a level: {token}"
            );
        }
    }
}
