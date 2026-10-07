//! Maps authored activities to the deterministic runtime (`assessment_engine::play`).
//!
//! The mapping lives here because `curriculum` already depends on
//! `assessment-engine` (the validators use `norm/1`), so the runtime cannot
//! depend on these types without a cycle. The same "caller maps" split as the
//! curriculum index (S4-02): the runtime is neutral, the authored types are
//! translated at the call site.
//!
//! Only the deterministic types map: `mcq`, `gap_fill`, `reorder`, `match`,
//! `dictation`, `reading_set`, `listening_set`, `error_correction`, and
//! `minimal_pairs` in listen mode. Productive and pronunciation activities are
//! not playable here; they are scored by the rubric or the phoneme engine.

use assessment_engine::play::{Item, Pair, Question};

use crate::{Activity, Localized, MinimalPairsMode};

/// Why an activity has no deterministic runtime.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NotPlayable {
    #[error("`{0}` is scored by the rubric or the phoneme engine, not deterministically")]
    ProductiveOrPron(String),
    #[error("`{0}` is `minimal_pairs` in say mode: it is scored by the phoneme engine")]
    MinimalPairsSayMode(String),
}

fn en(text: &Localized) -> String {
    text.en.clone()
}

impl Activity {
    /// This activity as a runtime item. An activity type that is not scored
    /// deterministically, and `minimal_pairs` in say mode, are refused with
    /// [`NotPlayable`] — never silently mapped to something playable.
    pub fn to_item(&self) -> Result<Item, NotPlayable> {
        let id = || self.common().id.clone();
        Ok(match self {
            Activity::Mcq {
                stem,
                options,
                answer_index,
                passage,
                audio_text,
                explanation,
                ..
            } => Item::Choice {
                stem: stem.clone(),
                options: options.clone(),
                answer_index: usize::from(*answer_index),
                passage: passage.clone(),
                audio_text: audio_text.clone(),
                explanation: en(explanation),
            },
            Activity::GapFill {
                text,
                answers,
                explanation,
                ..
            } => Item::GapFill {
                text: text.clone(),
                answers: answers.clone(),
                explanation: en(explanation),
            },
            Activity::Reorder {
                tokens,
                answer,
                explanation,
                ..
            } => Item::Reorder {
                tokens: tokens.clone(),
                answer: answer.clone(),
                explanation: explanation.as_ref().map(en),
            },
            Activity::Match { pairs, .. } => Item::Match {
                pairs: pairs
                    .iter()
                    .map(|pair| Pair {
                        left: pair.left.clone(),
                        right: pair.right.clone(),
                    })
                    .collect(),
            },
            Activity::Dictation {
                audio_text,
                accepted_answers,
                ..
            } => Item::Dictation {
                audio_text: audio_text.clone(),
                accepted: accepted_answers.clone(),
            },
            Activity::ReadingSet {
                passage, questions, ..
            } => Item::ReadingSet {
                passage: passage.clone(),
                questions: questions.iter().map(map_question).collect(),
            },
            Activity::ListeningSet {
                audio_text,
                replays_allowed,
                questions,
                ..
            } => Item::ListeningSet {
                // The schema allows `audio_text` or a dialogue reference. The
                // dialogue case has no text here; the caller plays the dialogue
                // and passes the transcript through this field.
                audio_text: audio_text.clone().unwrap_or_default(),
                replays_allowed: *replays_allowed,
                questions: questions.iter().map(map_question).collect(),
            },
            Activity::ErrorCorrection {
                sentence,
                accepted_answers,
                explanation,
                ..
            } => Item::ErrorCorrection {
                sentence: sentence.clone(),
                accepted: accepted_answers.clone(),
                explanation: en(explanation),
            },
            Activity::MinimalPairs { mode, pairs, .. } => {
                if *mode == MinimalPairsMode::SayBoth {
                    return Err(NotPlayable::MinimalPairsSayMode(id()));
                }
                Item::MinimalPairs {
                    pairs: pairs
                        .iter()
                        .map(|pair| Pair {
                            left: pair.a.clone(),
                            right: pair.b.clone(),
                        })
                        .collect(),
                }
            }
            other => return Err(NotPlayable::ProductiveOrPron(other.common().id.clone())),
        })
    }
}

fn map_question(question: &crate::Question) -> Question {
    Question {
        stem: question.stem.clone(),
        options: question.options.clone(),
        answer_index: usize::from(question.answer_index),
        explanation: en(&question.explanation),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::UnitLoader;

    const EXAMPLE: &str = include_str!("../../../curriculum/examples/a1-u01.example.json");

    // Helper for the tests below; clippy.toml only exempts `#[test]` bodies.
    #[allow(clippy::unwrap_used)]
    fn example() -> crate::Unit {
        UnitLoader::new().load_str(EXAMPLE).unwrap()
    }

    #[allow(clippy::unwrap_used)]
    fn activity(id: &str) -> Activity {
        example()
            .activities
            .into_iter()
            .find(|a| a.common().id == id)
            .unwrap()
    }

    #[test]
    fn every_deterministic_activity_of_the_example_unit_maps() {
        let unit = example();
        let mut mapped = 0;
        for activity in &unit.activities {
            match activity.common().scoring {
                Some(crate::Scoring::Deterministic) => {
                    let item = activity
                        .to_item()
                        .unwrap_or_else(|e| panic!("{} should map: {e}", activity.common().id));
                    // The mapping keeps the questions and answers intact.
                    match (&item, activity) {
                        (Item::Match { pairs }, Activity::Match { pairs: p, .. }) => {
                            assert_eq!(pairs.len(), p.len())
                        }
                        (Item::MinimalPairs { pairs }, Activity::MinimalPairs { pairs: p, .. }) => {
                            assert_eq!(pairs.len(), p.len())
                        }
                        _ => {}
                    }
                    mapped += 1;
                }
                Some(crate::Scoring::Rubric | crate::Scoring::Pron)
                | Some(crate::Scoring::None) => {
                    assert!(activity.to_item().is_err(), "{}", activity.common().id);
                }
                None => {}
            }
        }
        assert_eq!(
            mapped, 12,
            "the example unit has twelve deterministic activities"
        );
    }

    #[test]
    fn the_mapped_items_keep_the_authored_answers() {
        let item = activity("a01-listen-question").to_item().unwrap();
        let Item::Choice {
            options,
            answer_index,
            audio_text,
            ..
        } = item
        else {
            panic!("a01 is an mcq");
        };
        assert_eq!(options[answer_index], "name");
        assert!(audio_text.unwrap().contains("What's your name?"));

        let item = activity("a15-listen-set-putu").to_item().unwrap();
        let Item::ListeningSet {
            replays_allowed,
            questions,
            ..
        } = item
        else {
            panic!("a15 is a listening set");
        };
        assert_eq!(replays_allowed, 2);
        assert_eq!(questions.len(), 3);
        assert_eq!(questions[0].answer_index, 0);
        assert!(questions[0].options[questions[0].answer_index].contains("Bali"));

        let item = activity("a05-match-phrases").to_item().unwrap();
        let Item::Match { pairs } = item else {
            panic!("a05 is a match");
        };
        assert_eq!(pairs[0].left, "Good morning");
        assert_eq!(pairs[0].right, "Selamat pagi");
    }

    #[test]
    fn productive_pron_and_say_mode_activities_are_refused_with_their_reason() {
        for id in [
            "a07-read-aloud-thanks",
            "a10-speak-introduce",
            "a11-roleplay-classmate",
        ] {
            assert!(
                matches!(
                    activity(id).to_item(),
                    Err(NotPlayable::ProductiveOrPron(_))
                ),
                "{id}"
            );
        }
        // A minimal-pairs item in say mode is a pronunciation drill.
        let mut say = activity("a08-pairs-th");
        if let Activity::MinimalPairs { mode, .. } = &mut say {
            *mode = MinimalPairsMode::SayBoth;
        } else {
            panic!("a08 is minimal pairs");
        }
        assert!(matches!(
            say.to_item(),
            Err(NotPlayable::MinimalPairsSayMode(_))
        ));
    }

    #[test]
    fn a_mapped_item_scores_the_authored_correct_answer_full_marks() {
        use assessment_engine::play::{Answer, Item};
        // Walk the deterministic activities and answer each one correctly from
        // the authored data; every one must score 1.0.
        for activity in example().activities {
            let Some(crate::Scoring::Deterministic) = activity.common().scoring else {
                continue;
            };
            let item = activity.to_item().unwrap();
            let answer = match &item {
                Item::Choice { answer_index, .. } => Answer::Choices(vec![*answer_index]),
                Item::GapFill { answers, .. } => {
                    Answer::Gaps(answers.iter().map(|o| o[0].clone()).collect())
                }
                Item::Reorder { answer, .. } => Answer::Text(answer.clone()),
                Item::Match { pairs } => Answer::Pairs((0..pairs.len()).collect()),
                Item::Dictation { accepted, .. } => Answer::Text(accepted[0].clone()),
                Item::ReadingSet { questions, .. } | Item::ListeningSet { questions, .. } => {
                    Answer::Choices(questions.iter().map(|q| q.answer_index).collect())
                }
                Item::ErrorCorrection { accepted, .. } => Answer::Text(accepted[0].clone()),
                Item::MinimalPairs { pairs } => Answer::Heard {
                    played: vec![0; pairs.len()],
                    chosen: vec![0; pairs.len()],
                },
            };
            let scored = item.score(&answer).unwrap();
            assert_eq!(scored.score, 1.0, "{} ({:?})", activity.common().id, item);
            assert!(scored.success);
        }
    }
}
