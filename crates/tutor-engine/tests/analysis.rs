//! T2 turn analysis: prompt shape, the semantic filters (one test per rule),
//! the cadence, the notes buffer, and the reliability window. The live smoke
//! test is `examples/analysis_live.rs`, owner-run.
#![allow(clippy::unwrap_used)] // test helpers; clippy.toml only exempts #[test] functions

use serde_json::json;
use tutor_engine::{
    AnalysisCadence, AnalysisInput, AnalysisTurn, BATCH_SIZE, CONVERSATION_ERROR_CAP, DropCounts,
    InputMode, NotesForNextTurn, ObjectivePair, ReliabilityWindow, filter_output,
};

fn turn(seq: i64, learner_text: &str) -> AnalysisTurn {
    AnalysisTurn {
        turn_seq: seq,
        tutor_before: "Hello! What is your name?".to_owned(),
        learner_text: learner_text.to_owned(),
        tutor_reply: "Nice to meet you.".to_owned(),
    }
}

fn input(mode: InputMode, text: &str) -> AnalysisInput {
    AnalysisInput {
        level: curriculum::Level::A1,
        l1: "Indonesian".to_owned(),
        input_mode: mode,
        objectives: vec![ObjectivePair {
            id: "a1-u01/o1-greet".to_owned(),
            can_do: "Greet someone".to_owned(),
        }],
        target_language: vec!["hello".to_owned()],
        turns: vec![turn(1, text)],
    }
}

fn error(category: &str, quote: &str) -> serde_json::Value {
    json!({
        "category": category,
        "quote": quote,
        "correction": "x",
        "severity": "major",
        "addressed_in_reply": false
    })
}

fn evidence(objective_id: &str, status: &str, quote: &str) -> serde_json::Value {
    json!({ "objective_id": objective_id, "status": status, "quote": quote })
}

fn output(errors: Vec<serde_json::Value>, evidence: Vec<serde_json::Value>) -> serde_json::Value {
    json!({
        "turns": [{
            "turn_seq": 1,
            "errors": errors,
            "objective_evidence": evidence,
            "understood_tutor": "yes",
            "note_for_next_turn": "Ask about their hometown."
        }]
    })
}

#[test]
fn an_error_whose_quote_is_not_in_the_text_is_dropped() {
    let input = input(InputMode::Text, "I go to school yesterday.");
    let value = output(
        vec![error("verb_tense", "go"), error("article", "the school")],
        vec![],
    );
    let filtered = filter_output(&value, &input, 5).unwrap();
    assert_eq!(filtered.turns[0].errors.len(), 1);
    assert_eq!(filtered.turns[0].errors[0].quote, "go");
    assert_eq!(
        filtered.counts,
        DropCounts {
            produced: 2,
            dropped: 1
        }
    );
}

#[test]
fn the_quote_check_ignores_case_and_whitespace_runs() {
    let input = input(InputMode::Text, "I   went  Home.");
    let value = output(vec![error("verb_tense", "went home")], vec![]);
    let filtered = filter_output(&value, &input, 5).unwrap();
    assert_eq!(filtered.turns[0].errors.len(), 1);
    assert_eq!(filtered.counts.dropped, 0);
}

#[test]
fn an_empty_quote_never_matches() {
    let input = input(InputMode::Text, "Anything at all.");
    let value = output(vec![error("word_choice", "")], vec![]);
    let filtered = filter_output(&value, &input, 5).unwrap();
    assert!(filtered.turns[0].errors.is_empty());
    assert_eq!(filtered.counts.dropped, 1);
}

#[test]
fn voice_turns_drop_spelling_and_punctuation_errors() {
    let input = input(InputMode::Voice, "I go too school.");
    let value = output(
        vec![
            error("spelling", "too"),
            error("punctuation", "school."),
            error("preposition", "too school"),
        ],
        vec![],
    );
    let filtered = filter_output(&value, &input, 5).unwrap();
    let kept: Vec<&str> = filtered.turns[0]
        .errors
        .iter()
        .map(|e| e.category.as_str())
        .collect();
    assert_eq!(kept, ["preposition"]);
    assert_eq!(filtered.counts.dropped, 2);
}

#[test]
fn text_turns_keep_spelling_and_punctuation_errors() {
    let input = input(InputMode::Text, "I go too school.");
    let value = output(
        vec![error("spelling", "too"), error("punctuation", "school.")],
        vec![],
    );
    let filtered = filter_output(&value, &input, 5).unwrap();
    assert_eq!(filtered.turns[0].errors.len(), 2);
    assert_eq!(filtered.counts.dropped, 0);
}

#[test]
fn the_error_cap_keeps_the_model_order() {
    let input = input(InputMode::Text, "one two three four five six");
    let value = output(
        vec![
            error("word_choice", "one"),
            error("word_choice", "two"),
            error("word_choice", "three"),
            error("word_choice", "four"),
            error("word_choice", "five"),
            error("word_choice", "six"),
        ],
        vec![],
    );
    let filtered = filter_output(&value, &input, CONVERSATION_ERROR_CAP as usize).unwrap();
    let quotes: Vec<&str> = filtered.turns[0]
        .errors
        .iter()
        .map(|e| e.quote.as_str())
        .collect();
    assert_eq!(quotes, ["one", "two", "three", "four", "five"]);
    assert_eq!(filtered.counts.dropped, 1);
}

#[test]
fn evidence_for_an_unknown_objective_is_dropped() {
    let input = input(InputMode::Text, "Hello!");
    let value = output(
        vec![],
        vec![
            evidence("a1-u01/o1-greet", "demonstrated", "Hello"),
            evidence("a1-u01/o9-nope", "demonstrated", "Hello"),
        ],
    );
    let filtered = filter_output(&value, &input, 5).unwrap();
    assert_eq!(filtered.turns[0].objective_evidence.len(), 1);
    assert_eq!(
        filtered.turns[0].objective_evidence[0].objective_id,
        "a1-u01/o1-greet"
    );
    assert_eq!(filtered.counts.dropped, 1);
}

#[test]
fn evidence_with_an_empty_quote_survives_the_substring_filter() {
    let input = input(InputMode::Text, "Hello!");
    let value = output(
        vec![],
        vec![
            evidence("a1-u01/o1-greet", "not_demonstrated", ""),
            evidence("a1-u01/o1-greet", "partial", ""),
            evidence("a1-u01/o1-greet", "demonstrated", "Not in the text"),
        ],
    );
    // T2's filter drops evidence only when a *non-empty* quote is not a
    // substring; the prompt asks for an empty quote on not_demonstrated only,
    // and this filter is deliberately not stricter than the contract.
    let filtered = filter_output(&value, &input, 5).unwrap();
    let kept: Vec<&str> = filtered.turns[0]
        .objective_evidence
        .iter()
        .map(|e| e.status.as_str())
        .collect();
    assert_eq!(kept, ["not_demonstrated", "partial"]);
    assert_eq!(filtered.counts.dropped, 1);
}

#[test]
fn entries_for_unknown_turns_are_dropped_and_a_repeat_is_not_analysed_twice() {
    let input = input(InputMode::Text, "Hello!");
    let value = json!({
        "turns": [
            { "turn_seq": 1, "errors": [error("word_choice", "Hello")],
              "objective_evidence": [], "understood_tutor": "yes", "note_for_next_turn": "" },
            { "turn_seq": 7, "errors": [error("word_choice", "Hello")],
              "objective_evidence": [], "understood_tutor": "yes", "note_for_next_turn": "" },
            { "turn_seq": 1, "errors": [error("word_choice", "Hello")],
              "objective_evidence": [], "understood_tutor": "yes", "note_for_next_turn": "" }
        ]
    });
    let filtered = filter_output(&value, &input, 5).unwrap();
    assert_eq!(filtered.turns.len(), 1);
    assert_eq!(filtered.counts.produced, 3);
    assert_eq!(filtered.counts.dropped, 2);
}

#[test]
fn notes_skip_empty_ones_and_the_buffer_keeps_the_newest_three() {
    let one = input(InputMode::Text, "Hello!");
    let value = json!({
        "turns": [
            { "turn_seq": 1, "errors": [], "objective_evidence": [],
              "understood_tutor": "yes", "note_for_next_turn": "  " },
        ]
    });
    let filtered = filter_output(&value, &one, 5).unwrap();
    assert!(filtered.notes().is_empty());

    let turns: Vec<serde_json::Value> = [1, 2, 3, 4]
        .iter()
        .map(|seq| {
            json!({
                "turn_seq": seq, "errors": [], "objective_evidence": [],
                "understood_tutor": "yes", "note_for_next_turn": format!("note {seq}")
            })
        })
        .collect();
    let four = json!({ "turns": turns });
    let mut wide = input(InputMode::Text, "Hello!");
    wide.turns = vec![turn(1, "a"), turn(2, "b"), turn(3, "c"), turn(4, "d")];
    let filtered = filter_output(&four, &wide, 5).unwrap();
    assert_eq!(filtered.notes(), ["note 1", "note 2", "note 3", "note 4"]);

    let mut buffer = NotesForNextTurn::new();
    buffer.update(&filtered);
    assert_eq!(buffer.notes(), ["note 2", "note 3", "note 4"]);
}

#[test]
fn normal_cadence_analyses_every_turn() {
    let mut cadence = AnalysisCadence::new();
    assert!(!cadence.is_batched());
    cadence.enqueue(1);
    assert_eq!(cadence.due(), [1]);
    cadence.mark_analysed(&[1]);
    assert_eq!(cadence.pending(), 0);
    cadence.enqueue(2);
    cadence.enqueue(3);
    assert_eq!(cadence.due(), [2, 3]);
}

#[test]
fn after_a_rate_limit_one_call_carries_three_turns() {
    let mut cadence = AnalysisCadence::new();
    cadence.note_rate_limited();
    assert!(cadence.is_batched());

    cadence.enqueue(1);
    assert!(cadence.due().is_empty(), "one turn is not a batch yet");
    cadence.enqueue(2);
    assert!(cadence.due().is_empty(), "two turns are not a batch yet");
    cadence.enqueue(3);
    assert_eq!(cadence.due(), [1, 2, 3]);
    assert_eq!(BATCH_SIZE, 3);

    cadence.mark_analysed(&[1, 2, 3]);
    cadence.enqueue(4);
    cadence.enqueue(5);
    assert!(cadence.due().is_empty(), "the batch restarts empty");
    cadence.enqueue(6);
    assert_eq!(cadence.due(), [4, 5, 6]);
}

#[test]
fn a_session_end_flush_carries_everything_left_and_nothing_is_dropped() {
    let mut cadence = AnalysisCadence::new();
    cadence.note_rate_limited();
    cadence.enqueue(1);
    cadence.enqueue(2);
    cadence.enqueue(1); // a repeated number is kept once
    assert_eq!(cadence.flush(), [1, 2]);
    cadence.mark_analysed(&[1]);
    assert_eq!(cadence.flush(), [2]);
    cadence.mark_analysed(&[2]);
    assert!(cadence.flush().is_empty());
}

#[test]
fn the_reliability_window_flags_more_than_a_third_dropped() {
    let mut window = ReliabilityWindow::new();
    // Four analyses are below the floor even with everything dropped.
    for _ in 0..4 {
        window.record(DropCounts {
            produced: 4,
            dropped: 4,
        });
    }
    assert!(!window.unreliable());
    // A fifth tips the window over the floor and the share over a third.
    window.record(DropCounts {
        produced: 4,
        dropped: 4,
    });
    assert_eq!(window.analyses(), 5);
    assert!((window.dropped_share() - 1.0).abs() < f64::EPSILON);
    assert!(window.unreliable());
}

#[test]
fn the_reliability_window_keeps_only_the_last_twenty() {
    let mut window = ReliabilityWindow::new();
    for _ in 0..20 {
        window.record(DropCounts {
            produced: 10,
            dropped: 0,
        });
    }
    for _ in 0..20 {
        window.record(DropCounts {
            produced: 10,
            dropped: 10,
        });
    }
    assert_eq!(window.analyses(), 20);
    assert!((window.dropped_share() - 1.0).abs() < f64::EPSILON);
    assert!(window.unreliable());
    // An exactly one-third share is not "more than a third".
    let mut window = ReliabilityWindow::new();
    for _ in 0..5 {
        window.record(DropCounts {
            produced: 9,
            dropped: 3,
        });
    }
    assert!(!window.unreliable());
}
