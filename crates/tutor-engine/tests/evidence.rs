#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! The evidence recorder: the rows each kind of scorer leaves, and the counting
//! rule of the assessment spec, section 3.

mod common;

use assessment_engine::{Level, Origin};
use common::{make_profile, temp_db, test_clock};
use pron_engine::{
    Arpabet, FocusResult, Heard, Mode, Outcome, PhoneClass, PhonemeResult, UtteranceReport,
    WordResult,
};
use serde_json::{Value, json};
use speech::EngineInfo;
use storage::{AttemptStatus, Database, EvidenceKind, PronMode, Scorer};
use tutor_engine::{
    Alarm, CheckedRun, DeterministicRecord, Dimension, DimensionResult, DimensionStatus,
    EngineRole, EngineStamp, EvidenceRecorder, PointResult, PronRecord, RubricDimension,
    RubricMeta, RubricOutcome, RubricResult, Subject, WorkshopRubric,
};

struct Fixture {
    _dir: tempfile::TempDir,
    db: Database,
    recorder: EvidenceRecorder,
    profile_id: i64,
}

async fn fixture() -> Fixture {
    let (dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let recorder = EvidenceRecorder::new(db.clone(), test_clock());
    Fixture {
        _dir: dir,
        db,
        recorder,
        profile_id: profile.id,
    }
}

fn subject(f: &Fixture, origin: Origin, activity: &str, skill: &str) -> Subject {
    Subject::new(
        f.profile_id,
        None,
        Some("a1-u01".to_owned()),
        activity,
        "gap_fill",
        Level::A1,
        skill,
        origin,
        f.recorder.new_response_id(activity),
    )
}

fn gap_score() -> DeterministicRecord {
    DeterministicRecord {
        normalized: 0.5,
        raw: 1.0,
        max: 2.0,
        algorithm: "gap_fill/1",
        response_text: Some("am | from".to_owned()),
        details: json!({ "gaps": 2, "correct": 1 }),
        notes: vec!["gap 2: check the spelling of \"from\"".to_owned()],
    }
}

async fn evidence_of(db: &Database, attempt_id: i64) -> Vec<storage::Evidence> {
    db.evidence().for_attempt(attempt_id).await.unwrap()
}

fn metric(rows: &[storage::Evidence]) -> Value {
    rows.iter()
        .find(|e| e.kind == EvidenceKind::Metric)
        .and_then(|e| e.data.clone())
        .expect("a metric row")
}

#[tokio::test]
async fn a_deterministic_score_leaves_one_scored_counting_attempt_with_its_versions() {
    let f = fixture().await;
    let stamp = EngineStamp::new(
        EngineRole::Tts,
        &EngineInfo::new("sherpa-tts", "1.2").with_model_checksum("abc123"),
    );
    let subject = subject(&f, Origin::Authored, "a03-gap-am", "grammar").with_engine(stamp);
    let recorded = f
        .recorder
        .record_deterministic(&subject, &gap_score())
        .await
        .unwrap();
    assert_eq!(recorded.attempts.len(), 1);
    let attempt = &recorded.attempts[0];
    assert_eq!(attempt.scorer, Scorer::Deterministic);
    assert_eq!(attempt.scorer_version, "norm/1");
    assert_eq!(attempt.status, AttemptStatus::Scored);
    assert_eq!(attempt.normalized, Some(0.5));
    assert_eq!(attempt.confidence, Some(1.0));
    assert!(attempt.counts_toward_estimate);
    assert_eq!(attempt.skill, "grammar");
    assert_eq!(attempt.dimension, "overall");
    assert_eq!(attempt.unit_id.as_deref(), Some("a1-u01"));
    assert_eq!(attempt.response_id, subject.response_id);

    let rows = evidence_of(&f.db, attempt.id).await;
    let kinds: Vec<EvidenceKind> = rows.iter().map(|e| e.kind).collect();
    assert_eq!(
        kinds,
        [
            EvidenceKind::ResponseText,
            EvidenceKind::Metric,
            EvidenceKind::ScorerReason
        ]
    );
    assert_eq!(rows[0].content.as_deref(), Some("am | from"));
    let metric = metric(&rows);
    assert_eq!(metric["scorer"], "deterministic");
    assert_eq!(metric["norm_version"], "norm/1");
    assert_eq!(metric["algorithm_version"], "gap_fill/1");
    assert_eq!(metric["details"]["correct"], 1);
    assert_eq!(metric["engines"][0]["role"], "tts");
    assert_eq!(metric["engines"][0]["id"], "sherpa-tts");
    assert_eq!(metric["engines"][0]["version"], "1.2");
    assert_eq!(metric["engines"][0]["model_checksum"], "abc123");
}

#[tokio::test]
async fn generated_free_mode_and_practice_work_never_counts() {
    let f = fixture().await;
    let cases = [
        subject(&f, Origin::Generated, "gen-1", "grammar"),
        subject(&f, Origin::FreeMode, "reading-1", "reading"),
        subject(&f, Origin::Authored, "replay-1", "grammar").never_counts(),
    ];
    for subject in &cases {
        let recorded = f
            .recorder
            .record_deterministic(subject, &gap_score())
            .await
            .unwrap();
        let attempt = &recorded.attempts[0];
        assert_eq!(attempt.status, AttemptStatus::Scored);
        assert!(
            !attempt.counts_toward_estimate,
            "{:?} {}",
            subject.origin, subject.activity_id
        );
    }
    let all =
        f.db.attempts()
            .for_skill_since(f.profile_id, "grammar", &common::ts(0))
            .await
            .unwrap();
    assert!(all.iter().all(|a| !a.counts_toward_estimate));
}

#[tokio::test]
async fn a_score_outside_zero_to_one_is_refused() {
    let f = fixture().await;
    let mut bad = gap_score();
    bad.normalized = 1.2;
    let result = f
        .recorder
        .record_deterministic(&subject(&f, Origin::Authored, "x", "grammar"), &bad)
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn an_unscored_response_is_traceable_and_never_counts() {
    let f = fixture().await;
    let recorded = f
        .recorder
        .record_unscored(
            &subject(&f, Origin::Authored, "a09-shadow-dialogue", "pronunciation"),
            Scorer::Deterministic,
            "shadowing/1",
            "completion",
            AttemptStatus::Insufficient,
            "Shadowing has no scorer.",
            None,
            json!({ "lines": 4 }),
        )
        .await
        .unwrap();
    let attempt = &recorded.attempts[0];
    assert_eq!(attempt.status, AttemptStatus::Insufficient);
    assert_eq!(attempt.normalized, None);
    assert!(!attempt.counts_toward_estimate);
    let rows = evidence_of(&f.db, attempt.id).await;
    assert!(rows.iter().any(|e| e.kind == EvidenceKind::ScorerReason
        && e.content.as_deref() == Some("Shadowing has no scorer.")));
    assert_eq!(metric(&rows)["algorithm_version"], "unscored/1");

    let refused = f
        .recorder
        .record_unscored(
            &subject(&f, Origin::Authored, "x", "grammar"),
            Scorer::Deterministic,
            "x/1",
            "overall",
            AttemptStatus::Scored,
            "no",
            None,
            json!({}),
        )
        .await;
    assert!(refused.is_err(), "a scored status needs a score");
}

fn rubric() -> WorkshopRubric {
    WorkshopRubric {
        id: "rubric-a1-spoken-production".into(),
        version: 1,
        dimensions: [
            Dimension::TaskAchievement,
            Dimension::Range,
            Dimension::Accuracy,
            Dimension::Coherence,
        ]
        .into_iter()
        .map(|dimension| RubricDimension {
            dimension,
            bands: ["a", "b", "c", "d", "e"].map(String::from),
        })
        .collect(),
    }
}

fn dim(dimension: Dimension, band: Option<u8>) -> DimensionResult {
    DimensionResult {
        dimension,
        band,
        evidence_quotes: if band.is_some() {
            vec!["hello my name is Dewi".to_owned()]
        } else {
            Vec::new()
        },
        reason: "A reason.".to_owned(),
        status: if band.is_some() {
            DimensionStatus::Scored
        } else {
            DimensionStatus::NeedsReview
        },
        capped: false,
    }
}

fn run(bands: [Option<u8>; 4], alarms: Vec<Alarm>) -> CheckedRun {
    let dims = [
        Dimension::TaskAchievement,
        Dimension::Range,
        Dimension::Accuracy,
        Dimension::Coherence,
    ];
    CheckedRun {
        result: RubricResult {
            dimensions: dims
                .into_iter()
                .zip(bands)
                .map(|(d, b)| dim(d, b))
                .collect(),
            content_points: vec![PointResult {
                point: "a greeting".into(),
                covered: true,
                quote: "hello".into(),
            }],
            on_task: true,
            feedback_en: "Good start.".into(),
            feedback_l1: "Awal yang baik.".into(),
            confidence: 0.8,
            alarms,
            rejected: Vec::new(),
        },
        repaired: false,
    }
}

fn meta<'a>(rubric: &'a WorkshopRubric, model: Option<&'a str>, text: &'a str) -> RubricMeta<'a> {
    RubricMeta {
        rubric,
        model,
        input_mode: "voice",
        response_text: text,
        metrics: json!({ "words": 5 }),
        provider_qualified: true,
        ladder_level: Some(1),
    }
}

#[tokio::test]
async fn a_rubric_outcome_leaves_one_row_per_dimension_with_quotes_and_the_contract_version() {
    let f = fixture().await;
    let rubric = rubric();
    let outcome = RubricOutcome::join(
        &[run([Some(3), Some(2), Some(3), Some(3)], vec![])],
        true,
        Level::A1,
    );
    let stamp = EngineStamp::new(EngineRole::Stt, &EngineInfo::new("whisper", "base.en"));
    let subject = Subject::new(
        f.profile_id,
        None,
        Some("a1-u01".into()),
        "a10-speak-introduce",
        "guided_speaking",
        Level::A1,
        "speaking",
        Origin::Authored,
        f.recorder.new_response_id("a10"),
    )
    .with_engine(stamp);
    let recorded = f
        .recorder
        .record_rubric(
            &subject,
            &meta(&rubric, Some("test-model"), "hello my name is Dewi"),
            &outcome,
        )
        .await
        .unwrap();
    assert_eq!(recorded.attempts.len(), 4);
    assert!(
        recorded
            .attempts
            .iter()
            .all(|a| a.response_id == subject.response_id)
    );
    let range = recorded
        .attempts
        .iter()
        .find(|a| a.dimension == "range")
        .unwrap();
    assert_eq!(range.normalized, Some(0.5));
    assert_eq!(range.raw_score, Some(2.0));
    assert_eq!(range.max_score, Some(4.0));
    assert_eq!(range.confidence, Some(0.8));
    assert_eq!(range.scorer, Scorer::RubricLlm);
    assert_eq!(
        range.scorer_version,
        "rubric-a1-spoken-production/1+rubric_score/1+test-model"
    );
    assert!(range.counts_toward_estimate);

    let first = evidence_of(&f.db, recorded.attempts[0].id).await;
    assert_eq!(first[0].kind, EvidenceKind::ResponseText);
    let metric = metric(&first);
    assert_eq!(metric["algorithm_version"], "rubric_xcheck/1");
    assert_eq!(metric["details"]["contract_version"], "rubric_score/1");
    assert_eq!(metric["details"]["model"], "test-model");
    assert_eq!(metric["details"]["runs"], 1);
    assert_eq!(metric["engines"][0]["role"], "stt");
    let quotes = evidence_of(&f.db, range.id).await;
    assert!(
        quotes.iter().any(|e| e.kind == EvidenceKind::Quote
            && e.content.as_deref() == Some("hello my name is Dewi"))
    );
}

#[tokio::test]
async fn a_dimension_in_review_is_stored_without_a_score_and_never_counts() {
    let f = fixture().await;
    let rubric = rubric();
    let outcome = RubricOutcome::join(
        &[run([Some(3), None, Some(3), Some(3)], vec![])],
        true,
        Level::A1,
    );
    assert!(!outcome.is_complete());
    let subject = subject(&f, Origin::Authored, "a10", "speaking");
    let recorded = f
        .recorder
        .record_rubric(&subject, &meta(&rubric, Some("m"), "text"), &outcome)
        .await
        .unwrap();
    let review: Vec<_> = recorded
        .attempts
        .iter()
        .filter(|a| a.status == AttemptStatus::NeedsReview)
        .collect();
    assert_eq!(review.len(), 1);
    assert_eq!(review[0].dimension, "range");
    assert_eq!(review[0].normalized, None);
    assert!(!review[0].counts_toward_estimate);
}

#[tokio::test]
async fn free_mode_and_low_confidence_rubric_rows_do_not_count() {
    let f = fixture().await;
    let rubric = rubric();
    let outcome = RubricOutcome::join(
        &[run([Some(3), Some(3), Some(3), Some(3)], vec![])],
        false,
        Level::A1,
    );
    assert!((outcome.confidence - 0.5).abs() < 1e-9);
    let free = f
        .recorder
        .record_rubric(
            &subject(&f, Origin::FreeMode, "workshop", "writing"),
            &meta(&rubric, Some("m"), "text"),
            &outcome,
        )
        .await
        .unwrap();
    assert!(free.attempts.iter().all(|a| !a.counts_toward_estimate));

    // Not qualified, an alarm and a repair: 0.5 - 0.2 - 0.2 = 0.1, below the floor.
    let mut weak_run = run([Some(3), Some(3), Some(3), Some(3)], vec![Alarm::Range]);
    weak_run.repaired = true;
    let weak = RubricOutcome::join(&[weak_run], false, Level::A1);
    assert!((weak.confidence - 0.1).abs() < 1e-9);
    let stored = f
        .recorder
        .record_rubric(
            &subject(&f, Origin::Authored, "a10", "speaking"),
            &meta(&rubric, Some("m"), "text"),
            &weak,
        )
        .await
        .unwrap();
    assert!(
        stored
            .attempts
            .iter()
            .all(|a| a.status == AttemptStatus::Scored && !a.counts_toward_estimate)
    );
}

#[tokio::test]
async fn a_queued_response_is_pending_counts_provisionally_and_is_completed_later() {
    let f = fixture().await;
    let rubric = rubric();
    let subject = subject(&f, Origin::Authored, "a12-write-introduce", "writing");
    let queued = f
        .recorder
        .queue_rubric(
            &subject,
            &meta(&rubric, None, "Hello my name is Dewi"),
            &json!({ "kind": "rubric_response", "response_id": subject.response_id }),
        )
        .await
        .unwrap();
    assert_eq!(queued.attempts.len(), 4);
    assert!(
        queued
            .attempts
            .iter()
            .all(|a| a.status == AttemptStatus::PendingLlm && a.normalized.is_none())
    );
    assert!(queued.attempts[0].scorer_version.ends_with("+unscored"));
    assert_eq!(f.db.pending_scoring().oldest(10).await.unwrap().len(), 1);
    let waiting = evidence_of(&f.db, queued.attempts[0].id).await;
    assert_eq!(waiting[0].kind, EvidenceKind::ResponseText);

    let outcome = RubricOutcome::join(
        &[run([Some(4), Some(3), Some(3), Some(3)], vec![])],
        true,
        Level::A1,
    );
    f.recorder
        .complete_rubric(
            &queued.attempts,
            &subject,
            &meta(&rubric, Some("test-model"), "Hello my name is Dewi"),
            &outcome,
        )
        .await
        .unwrap();
    let done =
        f.db.attempts()
            .by_response(&subject.response_id)
            .await
            .unwrap();
    assert!(done.iter().all(|a| a.status == AttemptStatus::Scored));
    assert!(done.iter().all(|a| a.counts_toward_estimate));
    assert!(done[0].scorer_version.ends_with("+test-model"));
    assert_eq!(done[0].normalized, Some(1.0));

    // A late score below the confidence floor switches the flag off.
    let low = subject_with_id(&f, "a12-again");
    let queued = f
        .recorder
        .queue_rubric(&low, &meta(&rubric, None, "text"), &json!({}))
        .await
        .unwrap();
    assert!(queued.attempts.iter().all(|a| a.counts_toward_estimate));
    let mut weak_run = run([Some(3), Some(3), Some(3), Some(3)], vec![Alarm::Accuracy]);
    weak_run.repaired = true;
    let weak = RubricOutcome::join(&[weak_run], false, Level::A1);
    f.recorder
        .complete_rubric(
            &queued.attempts,
            &low,
            &meta(&rubric, Some("m"), "text"),
            &weak,
        )
        .await
        .unwrap();
    let after = f.db.attempts().by_response(&low.response_id).await.unwrap();
    assert!(after.iter().all(|a| !a.counts_toward_estimate));
}

fn subject_with_id(f: &Fixture, activity: &str) -> Subject {
    subject(f, Origin::Authored, activity, "writing")
}

fn phoneme(
    symbol: Arpabet,
    flagged: Option<bool>,
    gop: f32,
    heard: Option<Arpabet>,
) -> PhonemeResult {
    PhonemeResult {
        symbol,
        class: PhoneClass::Fricative,
        start_frame: 2,
        end_frame: 6,
        gop,
        score: flagged.map(|_| 0.5),
        flagged,
        heard: heard.map(|h| Heard {
            label: h.as_str().to_lowercase(),
            arpabet: Some(h),
            frames_won: 3,
        }),
    }
}

fn report(score: Option<f32>, mode: Mode) -> UtteranceReport {
    UtteranceReport {
        experimental: true,
        mode,
        counts_toward_assessment: mode.counts_toward_assessment(),
        outcome: Outcome::Aligned,
        words: vec![WordResult::Scored {
            text: "thank".to_owned(),
            pronunciation: vec![Arpabet::TH, Arpabet::AE, Arpabet::NG, Arpabet::K],
            start_frame: 0,
            end_frame: 20,
            phonemes: vec![
                phoneme(Arpabet::TH, Some(true), -4.0, Some(Arpabet::T)),
                phoneme(Arpabet::AE, Some(false), -0.2, None),
                phoneme(Arpabet::K, None, -0.5, None),
            ],
            score,
            flagged_phonemes: 1,
            adjacent_to_unchecked: false,
        }],
        utterance_score: score,
        words_scored: 1,
        words_not_checked: 0,
        scores_calibrated: score.is_some(),
        calibration_validated: false,
        focus: vec![FocusResult {
            symbol: Arpabet::TH,
            occurrences: 1,
            mean_gop: Some(-4.0),
            mean_score: score,
            flagged: 1,
        }],
        highlighted_words: Vec::new(),
        frames: 40,
    }
}

fn pron_record(report: &UtteranceReport, mode: PronMode) -> PronRecord<'_> {
    PronRecord {
        report,
        engine: EngineInfo::new("pron-engine", "0.0.0").with_model_checksum("deadbeef"),
        threshold_set_version: "unvalidated".to_owned(),
        reference_text: "Thank you.",
        mode,
        frame_ms: 20,
    }
}

#[tokio::test]
async fn a_pronunciation_drill_is_capped_at_point_six_and_traces_its_engine() {
    let f = fixture().await;
    let report = report(Some(0.9), Mode::Drill);
    let recorded = f
        .recorder
        .record_pron(
            &subject(
                &f,
                Origin::Authored,
                "a07-read-aloud-thanks",
                "pronunciation",
            ),
            &pron_record(&report, PronMode::Drill),
        )
        .await
        .unwrap();
    let attempt = &recorded.attempts[0];
    assert_eq!(attempt.scorer, Scorer::PronEngine);
    assert_eq!(attempt.status, AttemptStatus::Scored);
    assert!((attempt.normalized.unwrap() - 0.9).abs() < 1e-6);
    assert_eq!(attempt.confidence, Some(0.6));
    assert!(attempt.counts_toward_estimate);
    assert_eq!(
        attempt.scorer_version,
        "pron-engine 0.0.0+deadbeef+thresholds unvalidated"
    );
    let rows = evidence_of(&f.db, attempt.id).await;
    let metric = metric(&rows);
    assert_eq!(metric["scorer"], "pron_engine");
    assert_eq!(metric["algorithm_version"], "pron_gop/1");
    let pron_engine = metric["engines"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["role"] == "pron")
        .unwrap();
    assert_eq!(pron_engine["id"], "pron-engine");
    assert_eq!(pron_engine["model_checksum"], "deadbeef");
    assert_eq!(metric["details"]["experimental"], true);
    assert_eq!(metric["details"]["focus"][0]["symbol"], "TH");

    // Only phones a threshold decided on are stored: two of the three here.
    let stored = f.db.pron_results().for_attempt(attempt.id).await.unwrap();
    assert_eq!(stored.len(), 2);
    let th = stored.iter().find(|r| r.phone_expected == "TH").unwrap();
    assert!(th.flagged);
    assert_eq!(th.phone_heard.as_deref(), Some("T"));
    assert_eq!((th.start_ms, th.end_ms), (40, 120));
    assert_eq!(th.mode, PronMode::Drill);
    assert_eq!(th.word, "thank");
    assert!(
        stored
            .iter()
            .all(|r| r.engine_version == "pron-engine 0.0.0")
    );
}

#[tokio::test]
async fn a_report_without_a_score_is_insufficient_and_free_speech_never_counts() {
    let f = fixture().await;
    let unscored = report(None, Mode::Drill);
    let recorded = f
        .recorder
        .record_pron(
            &subject(&f, Origin::Authored, "a07", "pronunciation"),
            &pron_record(&unscored, PronMode::Drill),
        )
        .await
        .unwrap();
    let attempt = &recorded.attempts[0];
    assert_eq!(attempt.status, AttemptStatus::Insufficient);
    assert_eq!(attempt.normalized, None);
    assert!(
        !attempt.counts_toward_estimate,
        "no score, nothing to count"
    );
    let rows = evidence_of(&f.db, attempt.id).await;
    assert!(rows.iter().any(|e| {
        e.kind == EvidenceKind::ScorerReason
            && e.content
                .as_deref()
                .is_some_and(|c| c.contains("No score curve"))
    }));

    let free = report(
        Some(0.8),
        Mode::FreeSpeech {
            transcript_rejected: false,
        },
    );
    let recorded = f
        .recorder
        .record_pron(
            &subject(&f, Origin::Authored, "turn-1", "pronunciation"),
            &pron_record(&free, PronMode::FreeSpeech),
        )
        .await
        .unwrap();
    assert_eq!(recorded.attempts[0].status, AttemptStatus::Scored);
    assert!(
        !recorded.attempts[0].counts_toward_estimate,
        "free-speech hints never count, even from authored content"
    );
}
