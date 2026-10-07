//! S4-08 end to end: the example unit is completed from its first activity to
//! its checkpoint, against a real database.
//!
//! Every deterministic activity is answered from its authored data and stored
//! as a scored attempt; the two rubric activities are stored as `pending_llm`
//! attempts and queued in `pending_scoring`; the drills and the roleplay are
//! practice and leave no attempt; the checkpoint is read back from the stored
//! rows and passes provisionally while the productive items wait.
//!
//! The pronunciation drills (`read_aloud`, `minimal_pairs` say mode, and a
//! `shadowing` marked `pron`) need the pronunciation engine (S3-09), which is
//! not built on this line, so this walk records them as practice and the
//! submission API refuses them as unavailable rather than scoring a zero.
#![allow(clippy::unwrap_used)] // test helpers; clippy.toml only exempts #[test] functions

mod common;

use assessment_engine::play::{Answer, Item};
use common::TempDir;
use curriculum::{Scoring, Unit, UnitLoader};
use storage::{
    AttemptOrigin, AttemptStatus, Database, L1HelpMode, Level, NewAttempt, NewEvidence,
    NewPendingScoring, NewProfile, NewSession, Scorer, SessionKind, Skill, UiLanguage,
};
use tutor_engine::{
    PENDING_SCORER_VERSION, StoredAttempt, Submission, SubmissionOutcome, SubmitError,
    checkpoint_report, submit,
};

const EXAMPLE: &str = include_str!("../../../curriculum/examples/a1-u01.example.json");
const NOW: &str = "2026-10-08T08:00:00.000Z";

fn unit() -> Unit {
    UnitLoader::new().load_str(EXAMPLE).unwrap()
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

fn storage_level(level: curriculum::Level) -> Level {
    match level {
        curriculum::Level::A1 => Level::A1,
        curriculum::Level::A2 => Level::A2,
        curriculum::Level::B1 => Level::B1,
        curriculum::Level::B2 => Level::B2,
        curriculum::Level::C1 => Level::C1,
        curriculum::Level::C2 => Level::C2,
    }
}

fn storage_skill(dimension: &str) -> Skill {
    match dimension {
        "listening" => Skill::Listening,
        "reading" => Skill::Reading,
        "speaking" => Skill::Speaking,
        "writing" => Skill::Writing,
        "grammar" => Skill::Grammar,
        "vocabulary" => Skill::Vocabulary,
        "pronunciation" => Skill::Pronunciation,
        other => panic!("unknown dimension {other}"),
    }
}

#[tokio::test]
async fn the_example_unit_walks_from_first_activity_to_a_provisional_checkpoint() {
    let unit = unit();
    let dir = TempDir::new();
    let db = Database::open(dir.db_path()).await.unwrap();
    let profile = db
        .create_profile(NewProfile {
            display_name: "Learner".to_owned(),
            ui_language: UiLanguage::Id,
            l1: "id".to_owned(),
            l1_help_mode: L1HelpMode::Auto,
            created_at: NOW.to_owned(),
        })
        .await
        .unwrap();
    let session = db
        .create_session(NewSession {
            profile_id: profile.id,
            kind: SessionKind::Lesson,
            // The unit is not indexed in this test's database; a session's
            // unit_id has a foreign key to units, so the walk stores attempts
            // with the unit id but the session without it.
            unit_id: None,
            activity_id: None,
            mode: None,
            provider_profile_id: None,
            app_version: "0.0.0".to_owned(),
            started_at: NOW.to_owned(),
        })
        .await
        .unwrap();

    let mut scored = 0;
    let mut pending = 0;
    let mut practice = 0;
    for activity in &unit.activities {
        let id = activity.common().id.clone();
        let scoring = activity.common().scoring.unwrap();
        let outcome = match scoring {
            Scoring::Deterministic => {
                let item = activity.to_item().unwrap();
                let answer = correct_answer(&item);
                let outcome = submit(&unit, &id, Submission::Answer(answer)).unwrap();
                scored += 1;
                outcome
            }
            Scoring::Rubric => {
                // A spoken guided task on the voice channel, written otherwise.
                let (text, voice) = match activity {
                    curriculum::Activity::GuidedSpeaking(_) => (
                        "Hello. My name is Dewi. I am from Bandung.".to_owned(),
                        true,
                    ),
                    _ => ("Hello! I am Arif. I am from Makassar.".to_owned(), false),
                };
                let outcome = submit(&unit, &id, Submission::Production { text, voice }).unwrap();
                pending += 1;
                outcome
            }
            Scoring::None => {
                practice += 1;
                // A roleplay runs as a conversation (S4-05), turn by turn, so
                // it is practice here; anything else with scoring `none` is
                // acknowledged as done.
                if matches!(activity, curriculum::Activity::Roleplay { .. }) {
                    let error = submit(
                        &unit,
                        &id,
                        Submission::Production {
                            text: "Hello!".to_owned(),
                            voice: true,
                        },
                    )
                    .unwrap_err();
                    assert_eq!(error, SubmitError::Roleplay);
                } else {
                    submit(
                        &unit,
                        &id,
                        Submission::Production {
                            text: "done".to_owned(),
                            voice: false,
                        },
                    )
                    .unwrap();
                }
                continue;
            }
            Scoring::Pron => {
                // The engine is absent on this line: the walk must refuse it.
                let error = submit(
                    &unit,
                    &id,
                    Submission::Production {
                        text: "Thank you.".to_owned(),
                        voice: true,
                    },
                )
                .unwrap_err();
                assert_eq!(error, SubmitError::PronEngineUnavailable);
                practice += 1;
                continue;
            }
        };

        match outcome {
            SubmissionOutcome::Scored { outcome, skill } => {
                let response_id = format!("{id}@{NOW}#0");
                let attempt = db
                    .add_attempt(NewAttempt {
                        profile_id: profile.id,
                        session_id: Some(session.id),
                        unit_id: Some(unit.id.clone()),
                        activity_id: id.clone(),
                        activity_type: activity.type_str().to_owned(),
                        response_id,
                        origin: AttemptOrigin::Authored,
                        level: storage_level(unit.level),
                        skill: storage_skill(skill),
                        dimension: "overall".to_owned(),
                        scorer: Scorer::Deterministic,
                        scorer_version: "norm/1".to_owned(),
                        raw_score: Some(outcome.score),
                        max_score: Some(1.0),
                        normalized: Some(outcome.score),
                        confidence: Some(1.0),
                        status: AttemptStatus::Scored,
                        counts_toward_estimate: true,
                        created_at: NOW.to_owned(),
                    })
                    .await
                    .unwrap();
                db.add_evidence(NewEvidence {
                    attempt_id: attempt.id,
                    kind: storage::EvidenceKind::Metric,
                    content: None,
                    data_json: Some(
                        serde_json::json!({ "score": outcome.score, "success": outcome.success })
                            .to_string(),
                    ),
                    created_at: NOW.to_owned(),
                })
                .await
                .unwrap();
            }
            SubmissionOutcome::Pending { payload, skill } => {
                // One pending attempt per response, then the queue row.
                let attempt = db
                    .add_attempt(NewAttempt {
                        profile_id: profile.id,
                        session_id: Some(session.id),
                        unit_id: Some(unit.id.clone()),
                        activity_id: id.clone(),
                        activity_type: activity.type_str().to_owned(),
                        response_id: format!("{id}@{NOW}#0"),
                        origin: AttemptOrigin::Authored,
                        level: storage_level(unit.level),
                        skill: storage_skill(skill),
                        dimension: "overall".to_owned(),
                        scorer: Scorer::RubricLlm,
                        scorer_version: PENDING_SCORER_VERSION.to_owned(),
                        raw_score: None,
                        max_score: None,
                        normalized: None,
                        confidence: None,
                        status: AttemptStatus::PendingLlm,
                        // An authored response waiting for its score counts
                        // until the score arrives below the confidence floor.
                        counts_toward_estimate: true,
                        created_at: NOW.to_owned(),
                    })
                    .await
                    .unwrap();
                db.enqueue_pending_scoring(NewPendingScoring {
                    attempt_id: attempt.id,
                    payload_json: serde_json::to_string(&payload).unwrap(),
                    created_at: NOW.to_owned(),
                })
                .await
                .unwrap();
            }
            SubmissionOutcome::Practice => {}
        }
    }

    assert_eq!(scored, 12, "twelve deterministic activities");
    assert_eq!(pending, 2, "two rubric activities");
    assert_eq!(practice, 3, "read_aloud, shadowing and the roleplay");

    // The stored rows: twelve scored attempts, two waiting, no drill rows.
    let attempts = db.attempts_for_session(session.id).await.unwrap();
    assert_eq!(attempts.len(), 14);
    assert_eq!(
        attempts
            .iter()
            .filter(|a| a.status == AttemptStatus::Scored)
            .count(),
        12
    );
    assert_eq!(
        attempts
            .iter()
            .filter(|a| a.status == AttemptStatus::PendingLlm)
            .count(),
        2
    );
    // Every scored deterministic attempt counts toward an estimate; the two
    // pending ones wait in the queue with their payloads intact.
    let queue = db.pending_scoring(10).await.unwrap();
    assert_eq!(queue.len(), 2);
    let payload: serde_json::Value = serde_json::from_str(&queue[0].payload_json).unwrap();
    assert_eq!(payload["version"], "production_pending/1");
    assert_eq!(payload["unit_id"], "a1-u01");
    assert_eq!(
        payload["in_checkpoint"], true,
        "a10 is a checkpoint activity"
    );
    assert!(payload["response"].as_str().unwrap().contains("Dewi"));

    // The skills of the scored rows follow the spec's table.
    let mut by_skill = std::collections::HashMap::new();
    for attempt in attempts
        .iter()
        .filter(|a| a.status == AttemptStatus::Scored)
    {
        *by_skill.entry(attempt.skill.as_str()).or_insert(0) += 1;
    }
    assert_eq!(by_skill["listening"], 4, "a01, a06, a08, a15");
    assert_eq!(by_skill["reading"], 2, "a13, a14");
    assert_eq!(by_skill["grammar"], 2, "a03, a04");
    assert_eq!(by_skill["vocabulary"], 2, "a02, a05");
    assert_eq!(by_skill["writing"], 2, "a16, a17");

    // The checkpoint reads back from the stored rows: five deterministic
    // checkpoint activities scored, two productive ones waiting.
    let stored: Vec<StoredAttempt> = attempts
        .iter()
        .map(|attempt| StoredAttempt {
            activity_id: attempt.activity_id.clone(),
            scored: attempt.status == AttemptStatus::Scored && attempt.normalized.is_some(),
            normalized: attempt.normalized,
        })
        .collect();
    let report = checkpoint_report(&unit, &stored);
    assert!(
        report.unanswered.is_empty(),
        "every checkpoint item is answered"
    );
    assert_eq!(report.rows.len(), 7);
    assert!((report.outcome.mean - 1.0).abs() < 1e-9);
    assert!(report.outcome.passed);
    assert!(
        report.outcome.provisional,
        "the two productive items still wait for the rubric scorer"
    );

    // A second submission of an already answered activity is a new response,
    // not a second score for the same one: the caller decides to repeat, and
    // the walk's rows are unchanged.
    let again = submit(
        &unit,
        "a03-gap-am",
        Submission::Answer(Answer::Gaps(vec!["am".to_owned(), "at".to_owned()])),
    )
    .unwrap();
    let SubmissionOutcome::Scored { outcome, .. } = again else {
        panic!("a03 is deterministic");
    };
    assert_eq!(outcome.score, 0.5);
    assert_eq!(db.attempts_for_session(session.id).await.unwrap().len(), 14);

    // The database kept the learner text of a pending response in its payload
    // only; the scored rows carry metric evidence, no learner text.
    let evidence = db.evidence_for_attempt(attempts[0].id).await.unwrap();
    assert_eq!(evidence.len(), 1);
    assert_eq!(evidence[0].kind, storage::EvidenceKind::Metric);
    assert_eq!(evidence[0].content, None);
}

#[tokio::test]
async fn the_drills_and_the_roleplay_leave_no_attempt_rows() {
    let unit = unit();
    let dir = TempDir::new();
    let db = Database::open(dir.db_path()).await.unwrap();
    let profile = db
        .create_profile(NewProfile {
            display_name: "Learner".to_owned(),
            ui_language: UiLanguage::Id,
            l1: "id".to_owned(),
            l1_help_mode: L1HelpMode::Auto,
            created_at: NOW.to_owned(),
        })
        .await
        .unwrap();
    let session = db
        .create_session(NewSession {
            profile_id: profile.id,
            kind: SessionKind::Lesson,
            unit_id: None,
            activity_id: None,
            mode: None,
            provider_profile_id: None,
            app_version: "0.0.0".to_owned(),
            started_at: NOW.to_owned(),
        })
        .await
        .unwrap();

    // Practice and drills produce no attempts: scoring `none` is a `Practice`
    // outcome, and the walk never stores one.
    let outcome = submit(
        &unit,
        "a09-shadow-dialogue",
        Submission::Production {
            text: "done".to_owned(),
            voice: false,
        },
    )
    .unwrap();
    assert_eq!(outcome, SubmissionOutcome::Practice);
    // A roleplay is refused as a submission (it runs as a conversation).
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

    assert!(
        db.attempts_for_session(session.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(db.pending_scoring(10).await.unwrap().is_empty());
}
