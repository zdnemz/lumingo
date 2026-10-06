#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! The metrics of `assessment-engine` on the authored model answers of the
//! example unit: the three answers of each productive task must give figures
//! that rise from `below` to `above`, because the same answers are the scoring
//! anchors. The word list is a small one written for the test: the project ships
//! none yet.

mod common;

use std::collections::HashMap;

use assessment_engine::{Level, text_counts, vocabulary_profile, word_count};
use common::example_unit;
use curriculum::{Activity, Band, GuidedProduction};

fn a1_words() -> HashMap<String, Level> {
    [
        "hello",
        "my",
        "name",
        "is",
        "i",
        "am",
        "from",
        "and",
        "in",
        "nice",
        "to",
        "meet",
        "you",
        "good",
        "morning",
        "afternoon",
        "evening",
        "thank",
        "a",
        "new",
        "student",
        "class",
        "teacher",
        "this",
        "city",
        "big",
        "house",
        "friend",
    ]
    .into_iter()
    .map(|w| (w.to_owned(), Level::A1))
    .collect()
}

fn answers(production: &GuidedProduction) -> [&str; 3] {
    let text = |band: Band| {
        production
            .model_answers
            .iter()
            .find(|a| a.band == band)
            .map(|a| a.text.as_str())
            .expect("a model answer per band")
    };
    [text(Band::Below), text(Band::At), text(Band::Above)]
}

#[test]
fn the_model_answers_of_every_productive_task_give_figures_that_rise_from_below_to_above() {
    let unit = example_unit();
    let list = a1_words();
    let mut tasks = 0;
    for activity in &unit.activities {
        let (Activity::GuidedSpeaking(task) | Activity::GuidedWriting(task)) = activity else {
            continue;
        };
        tasks += 1;
        let [below, at, above] = answers(task);
        let counts: Vec<_> = [below, at, above].iter().map(|t| text_counts(t)).collect();
        let profiles: Vec<_> = [below, at, above]
            .iter()
            .map(|t| vocabulary_profile(t, Level::A1, &list))
            .collect();
        for pair in counts.windows(2) {
            assert!(pair[0].words < pair[1].words, "{}: words", task.id);
            assert!(
                pair[0].distinct_words < pair[1].distinct_words,
                "{}: distinct words",
                task.id
            );
        }
        for pair in profiles.windows(2) {
            assert!(
                pair[0].considered < pair[1].considered,
                "{}: running words",
                task.id
            );
            assert!(
                pair[0].distinct < pair[1].distinct,
                "{}: distinct considered words",
                task.id
            );
        }
        // The `at` answer is inside the task's own limits, the `below` one is under the minimum.
        assert!(word_count(at) >= task.min_words as usize);
        assert!(word_count(at) <= task.max_words as usize);
        assert!(word_count(below) < task.min_words as usize);
        // All three are A1 vocabulary: nothing on the list is above the unit's level.
        for profile in &profiles {
            assert_eq!(profile.above, 0, "{}", task.id);
        }
    }
    assert_eq!(
        tasks, 2,
        "the example unit has a speaking and a writing task"
    );
}
