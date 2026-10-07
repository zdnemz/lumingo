//! Playing a unit: the flow from an authored activity to what the caller stores.
//!
//! One [`Submission`] goes in, one [`SubmissionOutcome`] comes out.
//! Deterministic activities are scored through the `assessment-engine` runtime
//! (`assessment-engine::play`, S4-07) with normaliser `norm/1`. Productive ones
//! come back as a [`PendingProduction`] payload: the caller stores it as a
//! `pending_llm` attempt and queues it until the rubric scorer arrives (S5-03).
//! A pronunciation drill is refused because the engine (S3-09) is not built on
//! this line; an activity with scoring `none` is practice; a roleplay is a
//! conversation (S4-05), not a submission.
//!
//! Nothing here touches storage: the caller maps an outcome to rows, exactly as
//! it maps a turn analysis (S4-06). [`checkpoint_report`] reads the stored
//! responses back into the decision of `ASSESSMENT_SPEC` section 10: the mean
//! of the checkpoint's scored activities against the unit's pass mark,
//! provisional while any item is unscored.

use assessment_engine::checkpoint::{CheckpointItem, CheckpointOutcome, evaluate_checkpoint};
use assessment_engine::play::{self, Answer, Outcome};
use curriculum::{Activity, Level, NotPlayable, Scoring, Unit};
use serde::{Deserialize, Serialize};

/// The payload format of a productive response waiting for the rubric scorer.
pub const PRODUCTION_PENDING_VERSION: &str = "production_pending/1";

/// The `scorer_version` of a `pending_llm` attempt. The queue drain replaces it
/// with the rubric id, contract version and model name (S5-03).
pub const PENDING_SCORER_VERSION: &str = "pending/1";

/// What the learner handed in for one activity.
#[derive(Debug, Clone, PartialEq)]
pub enum Submission {
    /// The answer to a deterministic item, in the kind the item asks for.
    Answer(Answer),
    /// A productive response: its text, and whether it was spoken (a
    /// transcript) or written. A `mediation` files under the skill of the way
    /// it was answered.
    Production { text: String, voice: bool },
}

/// What one submission produced.
#[derive(Debug, Clone, PartialEq)]
pub enum SubmissionOutcome {
    /// A deterministic item, scored. The caller stores one scored attempt.
    Scored {
        outcome: Outcome,
        /// The evidence dimension (ASSESSMENT_SPEC section 2).
        skill: &'static str,
    },
    /// A productive response that waits for the rubric scorer. The caller
    /// stores a `pending_llm` attempt and queues [`PendingProduction`].
    Pending {
        payload: Box<PendingProduction>,
        /// The evidence dimension (ASSESSMENT_SPEC section 2).
        skill: &'static str,
    },
    /// The activity's scoring is `none`: practice with nothing to score.
    Practice,
}

/// Why a submission did not produce an outcome. None of these is a low score.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SubmitError {
    #[error("the unit has no activity `{0}`")]
    UnknownActivity(String),
    /// The schema requires `scoring` on every activity, so a missing value is a
    /// loader or content defect, not a learner mistake.
    #[error("the activity `{0}` has no `scoring` value; the schema requires one")]
    MissingScoring(String),
    /// The activity's fields do not fit its `scoring` value. The schema forbids
    /// this; it is a content defect.
    #[error("the activity `{id}` does not fit its scoring `{scoring}`")]
    ScoringMismatch { id: String, scoring: &'static str },
    /// A roleplay runs as a conversation (S4-05), turn by turn.
    #[error("a roleplay runs as a conversation, not as one submission")]
    Roleplay,
    /// The pronunciation engine (S3-09) is not built on this line, so a drill
    /// cannot be scored here. A missing engine is never a zero.
    #[error("a pronunciation drill needs the pronunciation engine (S3-09)")]
    PronEngineUnavailable,
    /// The activity has no deterministic runtime.
    #[error(transparent)]
    NotPlayable(#[from] NotPlayable),
    /// The answer is not the kind the item asked for: a caller bug.
    #[error(transparent)]
    Mismatch(#[from] play::AnswerMismatch),
    /// A productive response is text, never an item answer.
    #[error("a productive response needs its text, not an item answer")]
    ExpectedText,
    /// A deterministic item needs its answer kind, not a productive text.
    #[error("a deterministic item needs its answer kind, not a productive text")]
    ExpectedAnswer,
    #[error("the productive response is empty")]
    Empty,
}

/// A productive response waiting for the rubric scorer: everything the scorer
/// needs, so the queue can be drained with the rubric catalog open and nothing
/// else. `response` is the learner's own text and never leaves the device
/// except to the configured provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingProduction {
    pub version: String,
    pub unit_id: String,
    pub activity_id: String,
    pub activity_type: String,
    pub level: Level,
    /// The evidence dimension: `speaking` or `writing`.
    pub skill: String,
    pub rubric_id: String,
    /// The task prompt in English.
    pub prompt: String,
    pub content_points: Vec<String>,
    /// A mediation's source text; `None` for the other productive types.
    pub source_text: Option<String>,
    pub min_words: Option<u32>,
    pub max_words: Option<u32>,
    /// `voice` or `text`.
    pub input_mode: &'static str,
    /// True when the unit's checkpoint contains this activity: a checkpoint
    /// response is scored in two rubric runs (ASSESSMENT_SPEC section 5.2).
    pub in_checkpoint: bool,
    pub response: String,
}

/// Submits one response to one activity of `unit`.
///
/// The function scores whatever it is given: refusing a second attempt of an
/// activity is the caller's decision, because only the caller knows whether a
/// repeat is practice or a retake.
pub fn submit(
    unit: &Unit,
    activity_id: &str,
    submission: Submission,
) -> Result<SubmissionOutcome, SubmitError> {
    let activity = unit
        .activities
        .iter()
        .find(|activity| activity.common().id == activity_id)
        .ok_or_else(|| SubmitError::UnknownActivity(activity_id.to_owned()))?;
    let scoring = activity
        .common()
        .scoring
        .ok_or_else(|| SubmitError::MissingScoring(activity_id.to_owned()))?;

    if matches!(activity, Activity::Roleplay { .. }) {
        return Err(SubmitError::Roleplay);
    }

    match scoring {
        Scoring::Deterministic => {
            let Submission::Answer(answer) = submission else {
                return Err(SubmitError::ExpectedAnswer);
            };
            // `to_item` refuses the non-deterministic shapes, so a content
            // defect cannot slip through as a score.
            let item = activity.to_item()?;
            let outcome = item.score(&answer)?;
            Ok(SubmissionOutcome::Scored {
                outcome,
                skill: activity.evidence_skill(false),
            })
        }
        Scoring::Rubric => {
            let Submission::Production { text, voice } = submission else {
                return Err(SubmitError::ExpectedText);
            };
            if text.trim().is_empty() {
                return Err(SubmitError::Empty);
            }
            let skill = activity.evidence_skill(voice);
            let payload = production_payload(unit, activity, text, voice, skill)?;
            Ok(SubmissionOutcome::Pending {
                payload: Box::new(payload),
                skill,
            })
        }
        Scoring::Pron => Err(SubmitError::PronEngineUnavailable),
        Scoring::None => Ok(SubmissionOutcome::Practice),
    }
}

fn production_payload(
    unit: &Unit,
    activity: &Activity,
    text: String,
    voice: bool,
    skill: &'static str,
) -> Result<PendingProduction, SubmitError> {
    let (rubric_id, prompt, content_points, source_text, min_words, max_words) = match activity {
        Activity::GuidedSpeaking(production) | Activity::GuidedWriting(production) => (
            production.rubric_id.clone(),
            production.prompt.en.clone(),
            production.content_points.clone(),
            None,
            Some(production.min_words),
            Some(production.max_words),
        ),
        Activity::Mediation {
            source_text,
            task,
            rubric_id,
            ..
        } => (
            rubric_id.clone(),
            task.en.clone(),
            Vec::new(),
            Some(source_text.clone()),
            None,
            None,
        ),
        other => {
            return Err(SubmitError::ScoringMismatch {
                id: other.common().id.clone(),
                scoring: "rubric",
            });
        }
    };
    Ok(PendingProduction {
        version: PRODUCTION_PENDING_VERSION.to_owned(),
        unit_id: unit.id.clone(),
        activity_id: activity.common().id.clone(),
        activity_type: activity.type_str().to_owned(),
        level: unit.level,
        skill: skill.to_owned(),
        rubric_id,
        prompt,
        content_points,
        source_text,
        min_words,
        max_words,
        input_mode: if voice { "voice" } else { "text" },
        in_checkpoint: unit.checkpoint.activity_ids.contains(&activity.common().id),
        response: text,
    })
}

/// One stored attempt row of a session, as the checkpoint reads it. The caller
/// maps the rows of `attempts_for_session` one to one.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredAttempt {
    pub activity_id: String,
    /// True when the row is fully scored (`status = scored` and a score).
    pub scored: bool,
    /// The row's normalised score, when it has one.
    pub normalized: Option<f64>,
}

/// One checkpoint activity as it stands in the stored rows.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckpointRow {
    pub activity_id: String,
    /// The activity has at least one stored response.
    pub answered: bool,
    /// The mean of the activity's responses when every one is scored; `None`
    /// while anything waits (a productive response stored as `pending_llm`).
    pub score: Option<f64>,
}

/// The checkpoint decision, plus the rows behind it.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckpointReport {
    pub pass_mark: f64,
    pub rows: Vec<CheckpointRow>,
    /// Checkpoint activity ids with no stored response yet.
    pub unanswered: Vec<String>,
    pub outcome: CheckpointOutcome,
}

/// Reads the checkpoint of `unit` from the stored rows of one session. For each
/// checkpoint activity every stored response counts; the activity's score is
/// the mean of its responses when all of them are scored, and `None` while any
/// waits. Unanswered activities stay unscored, so the outcome is provisional
/// until the whole checkpoint is answered.
pub fn checkpoint_report(unit: &Unit, attempts: &[StoredAttempt]) -> CheckpointReport {
    let mut rows = Vec::with_capacity(unit.checkpoint.activity_ids.len());
    let mut unanswered = Vec::new();
    for id in &unit.checkpoint.activity_ids {
        let mine: Vec<&StoredAttempt> = attempts
            .iter()
            .filter(|attempt| &attempt.activity_id == id)
            .collect();
        if mine.is_empty() {
            unanswered.push(id.clone());
            rows.push(CheckpointRow {
                activity_id: id.clone(),
                answered: false,
                score: None,
            });
            continue;
        }
        let all_scored = mine
            .iter()
            .all(|attempt| attempt.scored && attempt.normalized.is_some());
        let score = all_scored.then(|| {
            mine.iter()
                .filter_map(|attempt| attempt.normalized)
                .sum::<f64>()
                / mine.len() as f64
        });
        rows.push(CheckpointRow {
            activity_id: id.clone(),
            answered: true,
            score,
        });
    }
    let items: Vec<CheckpointItem> = rows
        .iter()
        .map(|row| CheckpointItem { score: row.score })
        .collect();
    CheckpointReport {
        pass_mark: unit.checkpoint.pass_score,
        outcome: evaluate_checkpoint(&items, unit.checkpoint.pass_score),
        rows,
        unanswered,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assessment_engine::play::Item;
    use curriculum::{Common, GuidedProduction, Localized, MinimalPairsMode};

    const EXAMPLE: &str = include_str!("../../../curriculum/examples/a1-u01.example.json");

    // Helpers for the tests below; clippy.toml only exempts `#[test]` bodies.
    #[allow(clippy::unwrap_used)]
    fn example() -> Unit {
        curriculum::UnitLoader::new().load_str(EXAMPLE).unwrap()
    }

    #[allow(clippy::unwrap_used)]
    fn activity(id: &str) -> Activity {
        example()
            .activities
            .into_iter()
            .find(|a| a.common().id == id)
            .unwrap()
    }

    /// The authored correct answer of one mapped item.
    fn correct_answer(item: &Item) -> Answer {
        match item {
            Item::Choice { answer_index, .. } => Answer::Choices(vec![*answer_index]),
            Item::GapFill { answers, .. } => {
                Answer::Gaps(answers.iter().map(|options| options[0].clone()).collect())
            }
            Item::Reorder { answer, .. } => Answer::Text(answer.clone()),
            Item::Match { pairs } => Answer::Pairs((0..pairs.len()).collect()),
            Item::Dictation { accepted, .. } => Answer::Text(accepted[0].clone()),
            Item::ReadingSet { questions, .. } | Item::ListeningSet { questions, .. } => {
                Answer::Choices(
                    questions
                        .iter()
                        .map(|question| question.answer_index)
                        .collect(),
                )
            }
            Item::ErrorCorrection { accepted, .. } => Answer::Text(accepted[0].clone()),
            Item::MinimalPairs { pairs } => Answer::Heard {
                played: vec![0; pairs.len()],
                chosen: vec![0; pairs.len()],
            },
        }
    }

    #[allow(clippy::unwrap_used)]
    fn production(id: &str) -> GuidedProduction {
        match activity(id) {
            Activity::GuidedSpeaking(production) | Activity::GuidedWriting(production) => {
                production
            }
            other => panic!("{id} is not a guided production: {other:?}"),
        }
    }

    fn mediation_activity() -> Activity {
        Activity::Mediation {
            common: Common {
                id: "m1".to_owned(),
                skill: curriculum::Skill::Mediation,
                objective_ids: Vec::new(),
                instructions: Localized {
                    en: "Relay the message.".to_owned(),
                    id: None,
                },
                scoring: Some(Scoring::Rubric),
            },
            source_text: "The class starts at eight.".to_owned(),
            task: Localized {
                en: "Tell your friend when the class starts.".to_owned(),
                id: None,
            },
            rubric_id: "rubric-a1-mediation".to_owned(),
            model_answers: Vec::new(),
        }
    }

    #[test]
    fn every_deterministic_activity_scores_its_authored_answer() {
        let unit = example();
        let mut scored = 0;
        for activity in &unit.activities {
            if activity.common().scoring != Some(Scoring::Deterministic) {
                continue;
            }
            let item = activity
                .to_item()
                .unwrap_or_else(|e| panic!("{} should map: {e}", activity.common().id));
            let answer = correct_answer(&item);
            let outcome = submit(&unit, &activity.common().id, Submission::Answer(answer))
                .unwrap_or_else(|e| panic!("{} should submit: {e}", activity.common().id));
            match outcome {
                SubmissionOutcome::Scored { outcome, skill } => {
                    assert_eq!(outcome.score, 1.0, "{}", activity.common().id);
                    assert!(outcome.success);
                    assert_eq!(skill, activity.evidence_skill(false));
                }
                other => panic!("{} did not score: {other:?}", activity.common().id),
            }
            scored += 1;
        }
        assert_eq!(
            scored, 12,
            "the example unit has twelve deterministic activities"
        );
    }

    #[test]
    fn a_wrong_answer_scores_below_success_without_an_error() {
        let unit = example();
        // Both gaps wrong: 0 of 2.
        let outcome = submit(
            &unit,
            "a03-gap-am",
            Submission::Answer(Answer::Gaps(vec!["is".to_owned(), "at".to_owned()])),
        )
        .unwrap();
        let SubmissionOutcome::Scored { outcome, .. } = outcome else {
            panic!("a03 is deterministic");
        };
        assert_eq!(outcome.score, 0.0);
        assert!(!outcome.success);
        // A set answers by share: two of three questions right.
        let outcome = submit(
            &unit,
            "a15-listen-set-putu",
            Submission::Answer(Answer::Choices(vec![0, 1, 0])),
        )
        .unwrap();
        let SubmissionOutcome::Scored { outcome, .. } = outcome else {
            panic!("a15 is deterministic");
        };
        assert!((outcome.score - 2.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn the_submission_kind_must_match_the_activity() {
        let unit = example();
        // A productive text where an item answer is needed.
        let error = submit(
            &unit,
            "a02-greeting-by-time",
            Submission::Production {
                text: "Good evening.".to_owned(),
                voice: false,
            },
        )
        .unwrap_err();
        assert_eq!(error, SubmitError::ExpectedAnswer);
        // An item answer where a productive text is needed.
        let error = submit(
            &unit,
            "a10-speak-introduce",
            Submission::Answer(Answer::Text("Hello".to_owned())),
        )
        .unwrap_err();
        assert_eq!(error, SubmitError::ExpectedText);
        // The right kind but the wrong answer kind is a caller bug, not a zero.
        let error = submit(
            &unit,
            "a02-greeting-by-time",
            Submission::Answer(Answer::Text("Good evening.".to_owned())),
        )
        .unwrap_err();
        assert!(matches!(error, SubmitError::Mismatch(_)));
    }

    #[test]
    fn a_productive_response_becomes_a_pending_payload() {
        let unit = example();
        let text = "Hello. My name is Dewi. I'm from Bandung.";
        let outcome = submit(
            &unit,
            "a10-speak-introduce",
            Submission::Production {
                text: text.to_owned(),
                voice: true,
            },
        )
        .unwrap();
        let SubmissionOutcome::Pending { payload, skill } = outcome else {
            panic!("a10 is a rubric activity");
        };
        assert_eq!(skill, "speaking");
        assert_eq!(payload.version, PRODUCTION_PENDING_VERSION);
        assert_eq!(payload.unit_id, "a1-u01");
        assert_eq!(payload.activity_id, "a10-speak-introduce");
        assert_eq!(payload.activity_type, "guided_speaking");
        assert_eq!(payload.level, Level::A1);
        assert_eq!(payload.skill, "speaking");
        assert_eq!(
            payload.rubric_id,
            production("a10-speak-introduce").rubric_id
        );
        assert_eq!(payload.prompt, production("a10-speak-introduce").prompt.en);
        assert_eq!(payload.content_points.len(), 3);
        assert_eq!(payload.source_text, None);
        assert_eq!(payload.min_words, Some(5));
        assert_eq!(payload.max_words, Some(40));
        assert_eq!(payload.input_mode, "voice");
        assert_eq!(payload.response, text);
        assert!(payload.in_checkpoint, "a10 is a checkpoint activity");

        // A written response carries the written mode.
        let outcome = submit(
            &unit,
            "a12-write-introduce",
            Submission::Production {
                text: "Hello! I am Arif. I am from Makassar.".to_owned(),
                voice: false,
            },
        )
        .unwrap();
        let SubmissionOutcome::Pending { payload, skill } = outcome else {
            panic!("a12 is a rubric activity");
        };
        assert_eq!(skill, "writing");
        assert_eq!(payload.input_mode, "text");
        assert_eq!(payload.activity_type, "guided_writing");
    }

    #[test]
    fn a_mediation_files_under_the_skill_of_the_way_it_was_answered() {
        let mut unit = example();
        unit.activities = vec![mediation_activity()];
        let spoken = submit(
            &unit,
            "m1",
            Submission::Production {
                text: "The class starts at eight.".to_owned(),
                voice: true,
            },
        )
        .unwrap();
        let SubmissionOutcome::Pending { payload, skill } = spoken else {
            panic!("a mediation is rubric-scored");
        };
        assert_eq!(skill, "speaking");
        assert_eq!(payload.skill, "speaking");
        assert_eq!(
            payload.source_text.as_deref(),
            Some("The class starts at eight.")
        );
        assert_eq!(payload.min_words, None);

        let written = submit(
            &unit,
            "m1",
            Submission::Production {
                text: "Class starts at 8.".to_owned(),
                voice: false,
            },
        )
        .unwrap();
        let SubmissionOutcome::Pending { skill, .. } = written else {
            panic!("a mediation is rubric-scored");
        };
        assert_eq!(skill, "writing");
    }

    #[test]
    fn an_empty_productive_response_is_refused() {
        let unit = example();
        let error = submit(
            &unit,
            "a10-speak-introduce",
            Submission::Production {
                text: "   \n".to_owned(),
                voice: true,
            },
        )
        .unwrap_err();
        assert_eq!(error, SubmitError::Empty);
    }

    #[test]
    fn a_pronunciation_drill_is_refused_because_the_engine_is_not_on_this_line() {
        let unit = example();
        let error = submit(
            &unit,
            "a07-read-aloud-thanks",
            Submission::Production {
                text: "Thank you. Nice to meet you.".to_owned(),
                voice: true,
            },
        )
        .unwrap_err();
        assert_eq!(error, SubmitError::PronEngineUnavailable);

        // `minimal_pairs` in say mode: pron-scored is refused as unavailable;
        // a say-mode pair wrongly marked deterministic is a content defect.
        let mut say = activity("a08-pairs-th");
        if let Activity::MinimalPairs { mode, .. } = &mut say {
            *mode = MinimalPairsMode::SayBoth;
        } else {
            panic!("a08 is minimal pairs");
        }
        let mut say_pron = say.clone();
        if let Activity::MinimalPairs { common, .. } = &mut say_pron {
            common.scoring = Some(Scoring::Pron);
        }
        let mut unit = example();
        unit.activities = vec![say_pron];
        let error = submit(
            &unit,
            "a08-pairs-th",
            Submission::Production {
                text: "thank tank".to_owned(),
                voice: true,
            },
        )
        .unwrap_err();
        assert_eq!(error, SubmitError::PronEngineUnavailable);

        let mut unit = example();
        unit.activities = vec![say];
        let error = submit(
            &unit,
            "a08-pairs-th",
            Submission::Answer(Answer::Heard {
                played: vec![0, 0, 0],
                chosen: vec![0, 0, 0],
            }),
        )
        .unwrap_err();
        assert!(matches!(error, SubmitError::NotPlayable(_)));
    }

    #[test]
    fn a_roleplay_runs_as_a_conversation_and_scoring_none_is_practice() {
        let unit = example();
        let error = submit(
            &unit,
            "a11-roleplay-classmate",
            Submission::Production {
                text: "Hello!".to_owned(),
                voice: true,
            },
        )
        .unwrap_err();
        assert_eq!(error, SubmitError::Roleplay);

        let outcome = submit(
            &unit,
            "a09-shadow-dialogue",
            Submission::Production {
                text: "Hello!".to_owned(),
                voice: true,
            },
        )
        .unwrap();
        assert_eq!(outcome, SubmissionOutcome::Practice);
    }

    #[test]
    fn an_unknown_activity_is_refused() {
        let unit = example();
        let error = submit(
            &unit,
            "nope",
            Submission::Answer(Answer::Text("x".to_owned())),
        )
        .unwrap_err();
        assert_eq!(error, SubmitError::UnknownActivity("nope".to_owned()));
    }

    fn stored(activity_id: &str, score: Option<f64>) -> StoredAttempt {
        StoredAttempt {
            activity_id: activity_id.to_owned(),
            scored: score.is_some(),
            normalized: score,
        }
    }

    #[test]
    fn an_empty_run_passes_nothing_and_is_provisional() {
        let report = checkpoint_report(&example(), &[]);
        assert_eq!(report.pass_mark, 0.7);
        assert_eq!(report.rows.len(), 7);
        assert_eq!(report.unanswered.len(), 7);
        assert!(!report.outcome.passed);
        assert!(report.outcome.provisional);
        assert_eq!(report.outcome.mean, 0.0);
    }

    #[test]
    fn pending_productive_items_leave_the_decision_to_the_deterministic_ones() {
        let unit = example();
        let attempts = vec![
            stored("a02-greeting-by-time", Some(1.0)),
            stored("a03-gap-am", Some(1.0)),
            stored("a15-listen-set-putu", Some(1.0)),
            stored("a14-read-set-class-chat", Some(1.0)),
            stored("a16-fix-missing-am", Some(1.0)),
            stored("a10-speak-introduce", None),
            stored("a12-write-introduce", None),
        ];
        let report = checkpoint_report(&unit, &attempts);
        assert!(report.unanswered.is_empty());
        assert_eq!(report.outcome.mean, 1.0);
        assert!(report.outcome.passed);
        assert!(report.outcome.provisional, "two items still wait");
        let pending: Vec<&str> = report
            .rows
            .iter()
            .filter(|row| row.score.is_none())
            .map(|row| row.activity_id.as_str())
            .collect();
        assert_eq!(pending, ["a10-speak-introduce", "a12-write-introduce"]);
    }

    #[test]
    fn a_failed_checkpoint_does_not_pass_and_is_not_provisional() {
        let unit = example();
        let attempts: Vec<StoredAttempt> = unit
            .checkpoint
            .activity_ids
            .iter()
            .map(|id| stored(id, Some(0.5)))
            .collect();
        let report = checkpoint_report(&unit, &attempts);
        assert_eq!(report.outcome.mean, 0.5);
        assert!(!report.outcome.passed);
        assert!(!report.outcome.provisional);
    }

    #[test]
    fn an_unanswered_checkpoint_activity_keeps_the_result_provisional() {
        let unit = example();
        let attempts = vec![
            stored("a02-greeting-by-time", Some(1.0)),
            stored("a03-gap-am", Some(1.0)),
        ];
        let report = checkpoint_report(&unit, &attempts);
        assert_eq!(report.unanswered.len(), 5);
        assert_eq!(report.outcome.mean, 1.0);
        assert!(report.outcome.passed);
        assert!(report.outcome.provisional);
    }

    #[test]
    fn every_scored_response_of_an_activity_is_part_of_its_mean() {
        let unit = example();
        let attempts = vec![
            stored("a02-greeting-by-time", Some(1.0)),
            stored("a02-greeting-by-time", Some(0.0)),
        ];
        let report = checkpoint_report(&unit, &attempts);
        let row = report
            .rows
            .iter()
            .find(|row| row.activity_id == "a02-greeting-by-time")
            .unwrap();
        assert_eq!(row.score, Some(0.5));
        // One of the two responses still waiting keeps the activity unscored.
        let attempts = vec![
            stored("a02-greeting-by-time", Some(1.0)),
            stored("a02-greeting-by-time", None),
        ];
        let report = checkpoint_report(&unit, &attempts);
        let row = report
            .rows
            .iter()
            .find(|row| row.activity_id == "a02-greeting-by-time")
            .unwrap();
        assert!(row.answered);
        assert_eq!(row.score, None);
    }

    #[test]
    fn the_units_own_pass_mark_is_used() {
        let mut unit = example();
        unit.checkpoint.pass_score = 0.9;
        let attempts: Vec<StoredAttempt> = unit
            .checkpoint
            .activity_ids
            .iter()
            .map(|id| stored(id, Some(0.8)))
            .collect();
        let report = checkpoint_report(&unit, &attempts);
        assert_eq!(report.pass_mark, 0.9);
        assert!(!report.outcome.passed);
        let attempts: Vec<StoredAttempt> = unit
            .checkpoint
            .activity_ids
            .iter()
            .map(|id| stored(id, Some(1.0)))
            .collect();
        assert!(checkpoint_report(&unit, &attempts).outcome.passed);
    }
}
