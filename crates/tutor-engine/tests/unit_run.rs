#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! Playing the example unit from its first activity to its checkpoint: every
//! activity type, the rows each one leaves, offline behaviour and the backlog.

mod common;

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use assessment_engine::{Level, VoicedSpan};
use common::{ReactiveLlm, make_profile, temp_db, test_clock, ts};
use curriculum::{Activity, Scoring, Unit};
use llm_client::LlmClient;
use pron_engine::{Mode, Outcome, UtteranceReport};
use speech::{CancelFlag, EngineInfo};
use storage::{AttemptOrigin, AttemptStatus, Database, EvidenceKind, Scorer, UnitStatus};
use tokio_util::sync::CancellationToken;
use tutor_engine::{
    ActivityError, Clip, DrillError, DrillReport, DrillRequest, DrillScorer, EngineError,
    NoProvider, Response, ResultOutcome, RubricCatalog, SpokenResponse, UnitConfig, UnitEnv,
    UnitPlayer, UnscoredReason, checkpoint_report, ensure_indexed, settle_unit,
};

fn rubric_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../curriculum/catalogs/rubrics")
}

fn loaded_example() -> curriculum::LoadedUnit {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../curriculum/examples/a1-u01.example.json");
    curriculum::load_unit_file(&path).expect("the example unit loads")
}

#[derive(Clone, Copy)]
enum DrillMode {
    Score(f32),
    NoScore,
    Unavailable,
    Fail,
    WaitForCancel,
}

struct FakeDrill {
    mode: Mutex<DrillMode>,
    calls: AtomicUsize,
    seen: Mutex<Vec<DrillRequest>>,
}

impl FakeDrill {
    fn new(mode: DrillMode) -> Arc<Self> {
        Arc::new(Self {
            mode: Mutex::new(mode),
            calls: AtomicUsize::new(0),
            seen: Mutex::new(Vec::new()),
        })
    }
}

fn drill_report(score: Option<f32>) -> UtteranceReport {
    UtteranceReport {
        experimental: true,
        mode: Mode::Drill,
        counts_toward_assessment: true,
        outcome: Outcome::Aligned,
        words: Vec::new(),
        utterance_score: score,
        words_scored: usize::from(score.is_some()),
        words_not_checked: 0,
        scores_calibrated: score.is_some(),
        calibration_validated: false,
        focus: Vec::new(),
        highlighted_words: Vec::new(),
        frames: 50,
    }
}

impl DrillScorer for FakeDrill {
    fn engine(&self) -> EngineInfo {
        EngineInfo::new("fake-pron", "0.1").with_model_checksum("c0ffee")
    }

    fn threshold_set_version(&self) -> String {
        "unvalidated".to_owned()
    }

    fn score(
        &self,
        request: &DrillRequest,
        cancel: &CancelFlag,
    ) -> Result<DrillReport, DrillError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen.lock().unwrap().push(request.clone());
        let mode = *self.mode.lock().unwrap();
        let report = |score| {
            Ok(DrillReport {
                report: drill_report(score),
                frame_ms: 20,
            })
        };
        match mode {
            DrillMode::Score(score) => report(Some(score)),
            DrillMode::NoScore => report(None),
            DrillMode::Unavailable => Err(DrillError::Unavailable("no model installed".into())),
            DrillMode::Fail => Err(DrillError::Failed("bad audio".into())),
            DrillMode::WaitForCancel => {
                for _ in 0..5_000 {
                    if cancel.is_cancelled() {
                        return Err(DrillError::Cancelled);
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                report(Some(0.5))
            }
        }
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    db: Database,
    profile_id: i64,
    env: UnitEnv,
    unit: Unit,
    sha: String,
}

async fn fixture_with(
    client: Arc<dyn LlmClient>,
    drill: Option<Arc<dyn DrillScorer>>,
    rubrics: RubricCatalog,
) -> Fixture {
    let (dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let loaded = loaded_example();
    ensure_indexed(&db, &loaded.unit, &loaded.checksum, ts(1))
        .await
        .unwrap();
    let env = UnitEnv {
        client,
        db: db.clone(),
        clock: test_clock(),
        model: "test-model".into(),
        provider_profile_id: None,
        provider_qualified: true,
        grammar: None,
        word_levels: None,
        rubrics: Arc::new(rubrics),
        drill,
        tts: Some(EngineInfo::new("test-tts", "1").with_model_checksum("tts-sum")),
    };
    Fixture {
        _dir: dir,
        db,
        profile_id: profile.id,
        env,
        unit: loaded.unit,
        sha: loaded.checksum,
    }
}

async fn fixture(llm: &Arc<ReactiveLlm>, drill: Option<Arc<dyn DrillScorer>>) -> Fixture {
    fixture_with(
        llm.clone(),
        drill,
        RubricCatalog::load_dir(&rubric_dir()).expect("the rubrics load"),
    )
    .await
}

async fn player(f: &Fixture) -> UnitPlayer {
    player_of(f, f.unit.clone()).await
}

async fn player_of(f: &Fixture, unit: Unit) -> UnitPlayer {
    UnitPlayer::start(
        f.env.clone(),
        UnitConfig {
            profile_id: f.profile_id,
            first_language: "Indonesian".into(),
            app_version: "0.0.0-test".into(),
        },
        unit,
    )
    .await
    .unwrap()
}

fn cancel() -> CancellationToken {
    CancellationToken::new()
}

fn s(items: &[&str]) -> Vec<String> {
    items.iter().map(|i| (*i).to_owned()).collect()
}

fn text(t: &str) -> Response {
    Response::Text(t.to_owned())
}

fn clip() -> Clip {
    Clip {
        samples: vec![0.0; 16_000],
        says: None,
    }
}

fn spoken(transcript: &str) -> Response {
    Response::Spoken(SpokenResponse {
        transcript: transcript.to_owned(),
        spans: vec![
            VoicedSpan {
                start_ms: 200,
                end_ms: 2_200,
            },
            VoicedSpan {
                start_ms: 2_700,
                end_ms: 4_700,
            },
        ],
        stt: Some(EngineInfo::new("test-stt", "2").with_model_checksum("stt-sum")),
    })
}

/// The pairs of the unit's listening minimal pairs, answered right.
fn right_pairs(unit: &Unit, id: &str) -> Response {
    let Some(Activity::MinimalPairs(a)) = unit.activities.iter().find(|a| a.id() == id) else {
        panic!("minimal pairs");
    };
    Response::Picks(
        (0..a.pairs.len())
            .map(|i| Some(tutor_engine::minimal_pair_spoken(id, i)))
            .collect(),
    )
}

fn right_matches(unit: &Unit, id: &str) -> Response {
    let Some(Activity::Match(a)) = unit.activities.iter().find(|a| a.id() == id) else {
        panic!("a match");
    };
    let order = tutor_engine::match_display_order(id, a.pairs.len());
    Response::Picks(
        (0..a.pairs.len())
            .map(|left| order.iter().position(|pair| *pair == left))
            .collect(),
    )
}

/// Answers every activity of the example unit except the roleplay, which has
/// its own run. The read-aloud is answered with a recording.
async fn answer_all_but_roleplay(p: &mut UnitPlayer) {
    let unit = p.unit().clone();
    let cancel = cancel();
    let answers: Vec<(&str, Response)> = vec![
        ("a01-listen-question", Response::Choice(0)),
        ("a02-greeting-by-time", Response::Choice(1)),
        ("a03-gap-am", Response::Gaps(s(&["am", "from"]))),
        (
            "a04-reorder-name",
            Response::Order(s(&["My", "name", "is", "Dewi"])),
        ),
        (
            "a05-match-phrases",
            right_matches(&unit, "a05-match-phrases"),
        ),
        ("a06-dictation-from", text("Where are you from?")),
        ("a07-read-aloud-thanks", Response::Clips(vec![clip()])),
        ("a08-pairs-th", right_pairs(&unit, "a08-pairs-th")),
        ("a09-shadow-dialogue", Response::Done),
        (
            "a10-speak-introduce",
            spoken("Hello. My name is Dewi. I'm from Bandung."),
        ),
        (
            "a12-write-introduce",
            text("Hello! My name is Dewi. I am from Bandung. Nice to meet you."),
        ),
        ("a13-read-budi", Response::Choice(1)),
        (
            "a14-read-set-class-chat",
            Response::Picks(vec![Some(1), Some(2), Some(2), Some(0)]),
        ),
        (
            "a15-listen-set-putu",
            Response::Picks(vec![Some(0), Some(1), Some(1)]),
        ),
        ("a16-fix-missing-am", text("I am Dewi.")),
        ("a17-fix-missing-from", text("I am from Bandung.")),
    ];
    for (id, response) in answers {
        p.submit(id, response, &cancel).await.unwrap();
    }
}

async fn play_roleplay(p: &mut UnitPlayer) -> tutor_engine::ActivityResult {
    let cancel = cancel();
    let mut run = p.start_roleplay("a11-roleplay-classmate").unwrap();
    run.open(|_| {}, &cancel).await.unwrap();
    run.say("Hello, I am Dewi.", |_| {}, &cancel).await.unwrap();
    run.say("I am from Bandung. Where are you from?", |_| {}, &cancel)
        .await
        .unwrap();
    assert_eq!(run.turns_left(), 6);
    p.finish_roleplay(run, &cancel).await.unwrap()
}

#[tokio::test]
async fn the_example_unit_is_played_from_the_first_activity_to_the_checkpoint() {
    let llm = ReactiveLlm::new();
    let drill = FakeDrill::new(DrillMode::Score(0.8));
    let f = fixture(&llm, Some(drill.clone())).await;
    let mut p = player(&f).await;
    assert_eq!(p.activity_ids().len(), 17);
    answer_all_but_roleplay(&mut p).await;
    let roleplay = play_roleplay(&mut p).await;
    assert_eq!(roleplay.score, None, "scoring none: practice");
    assert!(matches!(
        roleplay.outcome,
        ResultOutcome::Unscored(UnscoredReason::NoScorer)
    ));
    assert_eq!(p.answered(), 17);

    let summary = p.finish().await.unwrap();
    assert_eq!(summary.status, UnitStatus::Passed);
    assert!(summary.checkpoint.outcome.passed);
    assert!(!summary.checkpoint.outcome.provisional);
    assert!(summary.checkpoint.unanswered.is_empty());
    assert_eq!(summary.checkpoint.rows.len(), 7);
    assert!(summary.checkpoint.rows.iter().all(|r| r.score.is_some()));
    // a02, a03, a15, a14 and a16 are right; a10 and a12 got a band of 3 of 4.
    let mean = (1.0 + 1.0 + 1.0 + 1.0 + 0.75 + 0.75 + 1.0) / 7.0;
    assert!((summary.checkpoint.outcome.mean - mean).abs() < 1e-9);

    let progress =
        f.db.unit_progress()
            .get(f.profile_id, "a1-u01")
            .await
            .unwrap()
            .unwrap();
    assert_eq!(progress.status, UnitStatus::Passed);
    assert!((progress.best_checkpoint.unwrap() - mean).abs() < 1e-9);

    // The rows: 12 objective items, 8 rubric dimensions, one drill, two unscored.
    let attempts = f.db.attempts().for_session(p.session_id()).await.unwrap();
    assert_eq!(attempts.len(), 23);
    assert!(attempts.iter().all(|a| a.origin == AttemptOrigin::Authored));
    assert!(
        attempts
            .iter()
            .all(|a| a.unit_id.as_deref() == Some("a1-u01"))
    );
    let by_scorer = |scorer: Scorer| attempts.iter().filter(|a| a.scorer == scorer).count();
    assert_eq!(by_scorer(Scorer::Deterministic), 12 + 2);
    assert_eq!(by_scorer(Scorer::RubricLlm), 8);
    assert_eq!(by_scorer(Scorer::PronEngine), 1);
    for attempt in &attempts {
        let scored = attempt.status == AttemptStatus::Scored;
        assert_eq!(
            attempt.counts_toward_estimate, scored,
            "{} {:?}",
            attempt.activity_id, attempt.status
        );
        assert!(
            scored == attempt.normalized.is_some(),
            "{}",
            attempt.activity_id
        );
    }
    let insufficient: Vec<&str> = attempts
        .iter()
        .filter(|a| a.status == AttemptStatus::Insufficient)
        .map(|a| a.activity_id.as_str())
        .collect();
    assert_eq!(
        insufficient,
        ["a09-shadow-dialogue", "a11-roleplay-classmate"]
    );

    // Skills follow the spec, not the author's tags.
    let skill_of = |id: &str| {
        attempts
            .iter()
            .find(|a| a.activity_id == id)
            .unwrap()
            .skill
            .clone()
    };
    assert_eq!(skill_of("a08-pairs-th"), "listening");
    assert_eq!(skill_of("a10-speak-introduce"), "speaking");
    assert_eq!(skill_of("a12-write-introduce"), "writing");
    assert_eq!(skill_of("a07-read-aloud-thanks"), "pronunciation");
    assert_eq!(skill_of("a13-read-budi"), "reading");

    // The checkpoint read back from the rows is the same decision.
    let again = checkpoint_report(
        &f.db,
        p.session_id(),
        &f.unit,
        &f.unit.checkpoint.activity_ids,
    )
    .await
    .unwrap();
    assert_eq!(again, summary.checkpoint);

    // The session is closed and summarised, with no learner text in the summary.
    let session = f.db.sessions().get(p.session_id()).await.unwrap().unwrap();
    assert_eq!(session.status, storage::SessionStatus::Completed);
    assert_eq!(session.summary.unwrap()["answered"], 17);

    // Two rubric runs for each of the two productive checkpoint items.
    assert_eq!(llm.rubric_calls(), 4);
    assert_eq!(drill.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn every_row_names_the_scorer_the_versions_and_the_engines_that_took_part() {
    let llm = ReactiveLlm::new();
    let drill = FakeDrill::new(DrillMode::Score(0.8));
    let f = fixture(&llm, Some(drill)).await;
    let mut p = player(&f).await;
    answer_all_but_roleplay(&mut p).await;
    let attempts = f.db.attempts().for_session(p.session_id()).await.unwrap();

    let mut metrics: std::collections::HashMap<String, serde_json::Value> =
        std::collections::HashMap::new();
    for attempt in &attempts {
        let rows = f.db.evidence().for_attempt(attempt.id).await.unwrap();
        if let Some(data) = rows
            .into_iter()
            .find(|e| e.kind == EvidenceKind::Metric)
            .and_then(|e| e.data)
        {
            metrics.insert(attempt.activity_id.clone(), data);
        }
    }
    let metric_of = |id: &str| metrics.get(id).cloned().expect("a metric row");
    // A listening item records the synthesiser the learner heard.
    for id in [
        "a01-listen-question",
        "a06-dictation-from",
        "a08-pairs-th",
        "a15-listen-set-putu",
    ] {
        let metric = metric_of(id);
        assert_eq!(metric["engines"][0]["role"], "tts", "{id}");
        assert_eq!(metric["engines"][0]["model_checksum"], "tts-sum", "{id}");
        assert_eq!(metric["norm_version"], "norm/1", "{id}");
    }
    // A reading item involved no speech engine.
    assert_eq!(
        metric_of("a14-read-set-class-chat")["engines"],
        serde_json::json!([])
    );
    assert_eq!(
        metric_of("a14-read-set-class-chat")["algorithm_version"],
        "reading_set/1"
    );
    // A spoken response records the recogniser, and its timing.
    let speaking = metric_of("a10-speak-introduce");
    assert_eq!(speaking["engines"][0]["role"], "stt");
    assert_eq!(speaking["engines"][0]["id"], "test-stt");
    assert_eq!(speaking["details"]["contract_version"], "rubric_score/1");
    assert_eq!(speaking["details"]["runs"], 2);
    assert_eq!(speaking["details"]["metrics"]["timing"]["pauses"], 1);
    assert_eq!(speaking["details"]["metrics"]["timing"]["voiced_ms"], 4000);
    // A drill records the pronunciation engine and its threshold set.
    let drill = metric_of("a07-read-aloud-thanks");
    assert_eq!(drill["scorer"], "pron_engine");
    assert!(
        drill["engines"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["role"] == "pron" && e["model_checksum"] == "c0ffee")
    );
    assert_eq!(drill["details"]["threshold_set_version"], "unvalidated");
    let written = metric_of("a12-write-introduce");
    assert_eq!(written["engines"], serde_json::json!([]));

    // The spoken turn keeps the voiced and silent time of the recording.
    let turns = f.db.turns().list(p.session_id()).await.unwrap();
    let voice = turns
        .iter()
        .find(|t| t.input_mode == storage::InputMode::Voice)
        .unwrap();
    assert_eq!((voice.speech_ms, voice.pause_ms), (Some(4000), Some(500)));
    assert_eq!(voice.word_count, Some(8));
}

#[tokio::test]
async fn offline_the_productive_items_wait_and_the_checkpoint_is_provisional_until_the_backlog_is_scored()
 {
    let llm = ReactiveLlm::new();
    llm.set_down(true);
    let f = fixture(&llm, None).await;
    let mut p = player(&f).await;
    answer_all_but_roleplay(&mut p).await;
    // The provider is down, so the roleplay cannot open; the run is left empty.
    let cancel = cancel();
    let mut run = p.start_roleplay("a11-roleplay-classmate").unwrap();
    let reply = run.open(|_| {}, &cancel).await.unwrap();
    assert_eq!(
        reply.outcome,
        tutor_engine::ReplyOutcome::ProviderUnavailable
    );
    p.finish_roleplay(run, &cancel).await.unwrap();

    for id in ["a10-speak-introduce", "a12-write-introduce"] {
        let result = p.result(id).unwrap();
        assert!(matches!(result.outcome, ResultOutcome::Queued), "{id}");
        assert_eq!(result.score, None);
    }
    // The read-aloud had no engine: stored as unscored with the reason, not as a zero.
    let read_aloud = p.result("a07-read-aloud-thanks").unwrap();
    assert!(matches!(
        read_aloud.outcome,
        ResultOutcome::Unscored(UnscoredReason::EngineUnavailable { .. })
    ));
    assert_eq!(read_aloud.score, None);

    let summary = p.finish().await.unwrap();
    assert!(
        summary.checkpoint.outcome.passed,
        "the objective items carry it"
    );
    assert!(summary.checkpoint.outcome.provisional);
    assert_eq!(
        summary.status,
        UnitStatus::InProgress,
        "a provisional pass is not a pass"
    );

    let attempts = f.db.attempts().for_session(p.session_id()).await.unwrap();
    let pending: Vec<_> = attempts
        .iter()
        .filter(|a| a.status == AttemptStatus::PendingLlm)
        .collect();
    assert_eq!(pending.len(), 8);
    assert!(pending.iter().all(|a| a.scorer == Scorer::RubricLlm));
    assert!(pending.iter().all(|a| a.counts_toward_estimate));
    assert_eq!(f.db.pending_scoring().oldest(10).await.unwrap().len(), 2);

    // The provider comes back and the backlog is scored.
    llm.set_down(false);
    let report = p.scorer().score_pending_backlog(&cancel).await.unwrap();
    assert_eq!(report.scored, 2);
    assert_eq!(report.remaining, 0);
    let settled = settle_unit(&f.db, ts(1_000), f.profile_id, p.session_id(), &f.unit)
        .await
        .unwrap();
    assert!(!settled.checkpoint.outcome.provisional);
    assert!(settled.checkpoint.outcome.passed);
    assert_eq!(settled.status, UnitStatus::Passed);
    let after = f.db.attempts().for_session(p.session_id()).await.unwrap();
    assert!(after.iter().all(|a| a.status != AttemptStatus::PendingLlm));
}

#[tokio::test]
async fn a_unit_with_no_provider_at_all_stores_everything_without_a_single_call_succeeding() {
    // The client of a learner who has set up no provider.
    let f = fixture_with(
        Arc::new(NoProvider),
        None,
        RubricCatalog::load_dir(&rubric_dir()).unwrap(),
    )
    .await;
    let mut p = player(&f).await;
    answer_all_but_roleplay(&mut p).await;
    assert!(matches!(
        p.result("a12-write-introduce").unwrap().outcome,
        ResultOutcome::Queued
    ));
    assert_eq!(
        p.result("a02-greeting-by-time").unwrap().score,
        Some(1.0),
        "objective items need no provider"
    );
}

#[tokio::test]
async fn an_activity_is_answered_once_per_run_and_an_unknown_one_is_refused() {
    let llm = ReactiveLlm::new();
    let f = fixture(&llm, None).await;
    let mut p = player(&f).await;
    p.submit("a02-greeting-by-time", Response::Choice(1), &cancel())
        .await
        .unwrap();
    let again = p
        .submit("a02-greeting-by-time", Response::Choice(0), &cancel())
        .await
        .unwrap_err();
    assert!(matches!(
        again,
        EngineError::Activity(ActivityError::AlreadyAnswered(_))
    ));
    let unknown = p
        .submit("a99-nothing", Response::Choice(0), &cancel())
        .await
        .unwrap_err();
    assert!(matches!(
        unknown,
        EngineError::Activity(ActivityError::UnknownActivity(_))
    ));
    // A refused response stores nothing.
    let attempts = f.db.attempts().for_session(p.session_id()).await.unwrap();
    assert_eq!(attempts.len(), 1);
    // A malformed response is refused and the activity can still be answered.
    let bad = p
        .submit("a03-gap-am", Response::Gaps(s(&["am"])), &cancel())
        .await
        .unwrap_err();
    assert!(matches!(
        bad,
        EngineError::Activity(ActivityError::WrongLength { .. })
    ));
    p.submit("a03-gap-am", Response::Gaps(s(&["am", "from"])), &cancel())
        .await
        .unwrap();
    assert_eq!(p.answered(), 2);
}

#[tokio::test]
async fn listening_sets_play_three_times_and_then_refuse() {
    let llm = ReactiveLlm::new();
    let f = fixture(&llm, None).await;
    let mut p = player(&f).await;
    let first = p.play_audio("a15-listen-set-putu").unwrap();
    assert_eq!((first.plays, first.replays_left), (1, Some(2)));
    assert!(first.lines[0].text.contains("Putu"));
    p.play_audio("a15-listen-set-putu").unwrap();
    let third = p.play_audio("a15-listen-set-putu").unwrap();
    assert_eq!((third.plays, third.replays_left), (3, Some(0)));
    assert!(matches!(
        p.play_audio("a15-listen-set-putu").unwrap_err(),
        EngineError::Activity(ActivityError::NoReplaysLeft { allowed: 2 })
    ));
    // The dictation has no limit, and an item with no audio has nothing to play.
    for _ in 0..5 {
        p.play_audio("a06-dictation-from").unwrap();
    }
    assert!(matches!(
        p.play_audio("a03-gap-am").unwrap_err(),
        EngineError::Activity(ActivityError::NoAudio(_))
    ));
    // The dialogue of the shadowing activity is played line by line.
    let lines = p.play_audio("a09-shadow-dialogue").unwrap().lines;
    assert!(lines.len() >= 4);
}

#[tokio::test]
async fn a_checkpoint_that_is_not_finished_cannot_be_settled() {
    let llm = ReactiveLlm::new();
    let f = fixture(&llm, None).await;
    let mut p = player(&f).await;
    p.submit("a02-greeting-by-time", Response::Choice(1), &cancel())
        .await
        .unwrap();
    let report = p.checkpoint().await.unwrap();
    assert_eq!(report.unanswered.len(), 6);
    assert!(matches!(
        p.finish().await.unwrap_err(),
        EngineError::Refused(_)
    ));
    // The unit stays in progress, and the session stays open.
    let progress =
        f.db.unit_progress()
            .get(f.profile_id, "a1-u01")
            .await
            .unwrap()
            .unwrap();
    assert_eq!(progress.status, UnitStatus::InProgress);
    p.abort().await.unwrap();
    let session = f.db.sessions().get(p.session_id()).await.unwrap().unwrap();
    assert_eq!(session.status, storage::SessionStatus::Aborted);
}

#[tokio::test]
async fn an_objective_is_a_moving_average_of_its_activities() {
    let llm = ReactiveLlm::new();
    let f = fixture(&llm, None).await;
    let mut p = player(&f).await;
    // o2-introduce is served by a03 (gap fill), a04 (reorder), a10, a12 and others.
    p.submit("a03-gap-am", Response::Gaps(s(&["am", "from"])), &cancel())
        .await
        .unwrap();
    p.submit(
        "a04-reorder-name",
        Response::Order(s(&["name", "My", "Dewi", "is"])),
        &cancel(),
    )
    .await
    .unwrap();
    let mastery =
        f.db.mastery()
            .get(f.profile_id, "a1-u01/o2-introduce")
            .await
            .unwrap()
            .unwrap();
    assert_eq!(mastery.attempts, 2);
    // 1.0 first, then a miss: 0.3 * 0 + 0.7 * 1.0.
    assert!((mastery.mastery - 0.7).abs() < 1e-9, "{}", mastery.mastery);
    assert!(mastery.last_attempt_at.is_some());
}

#[tokio::test]
async fn a_response_with_a_missing_rubric_is_stored_unscored_and_says_which() {
    let llm = ReactiveLlm::new();
    let f = fixture_with(llm.clone(), None, RubricCatalog::default()).await;
    let mut p = player(&f).await;
    let result = p
        .submit(
            "a12-write-introduce",
            text("Hello! My name is Dewi. I am from Bandung."),
            &cancel(),
        )
        .await
        .unwrap();
    assert!(matches!(
        result.outcome,
        ResultOutcome::Unscored(UnscoredReason::RubricMissing { ref rubric_id })
            if rubric_id == "rubric-a1-written-production"
    ));
    assert_eq!(llm.rubric_calls(), 0);
    let attempts = f.db.attempts().for_session(p.session_id()).await.unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].status, AttemptStatus::Insufficient);
    assert!(!attempts[0].counts_toward_estimate);
}

#[tokio::test]
async fn productive_responses_of_the_wrong_shape_or_empty_are_refused() {
    let llm = ReactiveLlm::new();
    let f = fixture(&llm, None).await;
    let mut p = player(&f).await;
    for (id, response) in [
        ("a10-speak-introduce", text("typed instead of spoken")),
        ("a12-write-introduce", spoken("spoken instead of typed")),
        ("a12-write-introduce", text("   ")),
        ("a10-speak-introduce", spoken("")),
    ] {
        assert!(p.submit(id, response, &cancel()).await.is_err(), "{id}");
    }
    assert_eq!(llm.rubric_calls(), 0);
    assert!(
        f.db.attempts()
            .for_session(p.session_id())
            .await
            .unwrap()
            .is_empty()
    );
    // Malformed voiced spans are refused rather than guessed at.
    let overlapping = Response::Spoken(SpokenResponse {
        transcript: "Hello. My name is Dewi.".into(),
        spans: vec![
            VoicedSpan {
                start_ms: 0,
                end_ms: 100,
            },
            VoicedSpan {
                start_ms: 50,
                end_ms: 200,
            },
        ],
        stt: None,
    });
    assert!(
        p.submit("a10-speak-introduce", overlapping, &cancel())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn drills_score_each_clip_against_its_reference_and_store_one_attempt_per_clip() {
    let llm = ReactiveLlm::new();
    let drill = FakeDrill::new(DrillMode::Score(0.9));
    let f = fixture(&llm, Some(drill.clone())).await;
    // A say-mode minimal-pairs drill and a scored shadowing, built from the example.
    let mut unit = f.unit.clone();
    for activity in &mut unit.activities {
        match activity {
            Activity::MinimalPairs(a) if a.id == "a08-pairs-th" => {
                a.mode = curriculum::MinimalPairsMode::SayBoth;
                a.scoring = Scoring::Pron;
            }
            Activity::Shadowing(a) => a.scoring = Scoring::Pron,
            _ => {}
        }
    }
    let mut p = player_of(&f, unit.clone()).await;
    let pairs = p
        .submit(
            "a08-pairs-th",
            Response::Clips(vec![clip(), clip(), clip()]),
            &cancel(),
        )
        .await
        .unwrap();
    assert_eq!(pairs.response_ids.len(), 3);
    assert!((pairs.score.unwrap() - 0.9).abs() < 1e-6);
    assert_eq!(pairs.confidence, Some(0.6));
    let seen: Vec<String> = drill
        .seen
        .lock()
        .unwrap()
        .iter()
        .map(|r| r.reference_text.clone())
        .collect();
    assert_eq!(seen, ["thank tank", "three tree", "think sink"]);

    let lines = match unit
        .activities
        .iter()
        .find(|a| a.id() == "a09-shadow-dialogue")
    {
        Some(Activity::Shadowing(_)) => p.present("a09-shadow-dialogue").unwrap(),
        _ => panic!("shadowing"),
    };
    let tutor_engine::Body::Shadowing { lines, .. } = lines.body else {
        panic!("shadowing body");
    };
    let shadow = p
        .submit(
            "a09-shadow-dialogue",
            Response::Clips(lines.iter().map(|_| clip()).collect()),
            &cancel(),
        )
        .await
        .unwrap();
    assert_eq!(shadow.response_ids.len(), lines.len());
    let attempts = f.db.attempts().for_session(p.session_id()).await.unwrap();
    assert_eq!(attempts.len(), 3 + lines.len());
    assert!(attempts.iter().all(|a| a.scorer == Scorer::PronEngine));
    assert!(attempts.iter().all(|a| a.skill == "pronunciation"));

    // The wrong number of clips is refused before the engine is called.
    let before = drill.calls.load(Ordering::SeqCst);
    let mut again = player_of(&f, unit).await;
    let error = again
        .submit("a08-pairs-th", Response::Clips(vec![clip()]), &cancel())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        EngineError::Activity(ActivityError::WrongLength {
            expected: 3,
            got: 1
        })
    ));
    assert_eq!(drill.calls.load(Ordering::SeqCst), before);
}

#[tokio::test]
async fn a_drill_whose_engine_is_missing_fails_or_returns_no_score_is_stored_as_not_scored() {
    for mode in [DrillMode::Unavailable, DrillMode::Fail, DrillMode::NoScore] {
        let llm = ReactiveLlm::new();
        let drill = FakeDrill::new(mode);
        let f = fixture(&llm, Some(drill)).await;
        let mut p = player(&f).await;
        let result = p
            .submit(
                "a07-read-aloud-thanks",
                Response::Clips(vec![clip()]),
                &cancel(),
            )
            .await
            .unwrap();
        assert_eq!(result.score, None);
        let attempts = f.db.attempts().for_session(p.session_id()).await.unwrap();
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].status, AttemptStatus::Insufficient);
        assert_eq!(attempts[0].normalized, None);
        assert!(!attempts[0].counts_toward_estimate);
    }
}

#[tokio::test]
async fn a_cancelled_drill_stops_the_engine_and_is_an_error() {
    let llm = ReactiveLlm::new();
    let drill = FakeDrill::new(DrillMode::WaitForCancel);
    let f = fixture(&llm, Some(drill)).await;
    let mut p = player(&f).await;
    let token = CancellationToken::new();
    let stopper = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        stopper.cancel();
    });
    let error = p
        .submit(
            "a07-read-aloud-thanks",
            Response::Clips(vec![clip()]),
            &token,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, EngineError::Cancelled));
    assert!(p.result("a07-read-aloud-thanks").is_none());
}

#[tokio::test]
async fn a_drill_with_no_scorer_needs_no_recording() {
    let llm = ReactiveLlm::new();
    let f = fixture(&llm, None).await;
    let mut p = player(&f).await;
    let result = p
        .submit("a09-shadow-dialogue", Response::Done, &cancel())
        .await
        .unwrap();
    assert!(matches!(
        result.outcome,
        ResultOutcome::Unscored(UnscoredReason::NoScorer)
    ));
    let wrong = p
        .submit("a07-read-aloud-thanks", Response::Done, &cancel())
        .await
        .unwrap_err();
    assert!(matches!(
        wrong,
        EngineError::Activity(ActivityError::WrongKind { .. })
    ));
}

#[tokio::test]
async fn a_roleplay_has_a_turn_limit_and_its_turns_belong_to_the_unit_session() {
    let llm = ReactiveLlm::new();
    let f = fixture(&llm, None).await;
    let mut unit = f.unit.clone();
    for activity in &mut unit.activities {
        if let Activity::Roleplay(a) = activity {
            a.max_turns = 2;
        }
    }
    let mut p = player_of(&f, unit).await;
    let cancel = cancel();
    let mut run = p.start_roleplay("a11-roleplay-classmate").unwrap();
    let opening = run.open(|_| {}, &cancel).await.unwrap();
    assert!(opening.text.starts_with("Nice to meet you"));
    run.say("Hello, I am Dewi.", |_| {}, &cancel).await.unwrap();
    run.say("I am from Bandung.", |_| {}, &cancel)
        .await
        .unwrap();
    assert_eq!(run.turns_left(), 0);
    let refused = run.say("And you?", |_| {}, &cancel).await.unwrap_err();
    assert!(matches!(
        refused,
        EngineError::Activity(ActivityError::NoTurnsLeft { max: 2 })
    ));
    // The system prompt carries the scenario of the unit and its target language.
    let seen = llm.text_seen.lock().unwrap().clone();
    assert!(seen[0].system.contains("Sam, a friendly new classmate"));
    assert!(seen[0].system.contains("Unit: "));
    assert!(seen[0].system.contains("Mode: fluency"));
    let result = p.finish_roleplay(run, &cancel).await.unwrap();
    assert!(matches!(result.outcome, ResultOutcome::Unscored(_)));
    // The tutor's turns and the learner's turns are stored with the unit session,
    // which the roleplay did not close.
    let turns = f.db.turns().list(p.session_id()).await.unwrap();
    assert_eq!(
        turns.len(),
        5,
        "an opening, two learner turns and their replies"
    );
    let session = f.db.sessions().get(p.session_id()).await.unwrap().unwrap();
    assert_eq!(session.status, storage::SessionStatus::Active);
    assert_eq!(session.kind, storage::SessionKind::Lesson);
    // Roleplay chat is not stored as free-mode attempts.
    let attempts = f.db.attempts().for_session(p.session_id()).await.unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].origin, AttemptOrigin::Authored);
    assert_eq!(attempts[0].activity_id, "a11-roleplay-classmate");
}

#[tokio::test]
async fn a_scored_roleplay_is_scored_with_the_interaction_rubric_of_the_level() {
    let llm = ReactiveLlm::new();
    let f = fixture(&llm, None).await;
    let mut unit = f.unit.clone();
    for activity in &mut unit.activities {
        if let Activity::Roleplay(a) = activity {
            a.scoring = Scoring::Rubric;
        }
    }
    let mut p = player_of(&f, unit).await;
    let result = play_roleplay(&mut p).await;
    let ResultOutcome::Rubric(outcome) = &result.outcome else {
        panic!("scored: {:?}", result.outcome);
    };
    assert_eq!(outcome.dimensions.len(), 5, "an interaction dimension");
    assert_eq!(result.score, Some(0.75));
    let attempts = f.db.attempts().for_session(p.session_id()).await.unwrap();
    let rubric_rows: Vec<_> = attempts
        .iter()
        .filter(|a| a.scorer == Scorer::RubricLlm)
        .collect();
    assert_eq!(rubric_rows.len(), 5);
    assert!(rubric_rows.iter().all(|a| a.skill == "speaking"));
    assert!(rubric_rows.iter().all(|a| a.counts_toward_estimate));
    assert!(
        rubric_rows[0]
            .scorer_version
            .starts_with("rubric-a1-spoken-interaction/1")
    );
    // The scored text was the learner's two lines.
    let call = llm
        .structured_seen
        .lock()
        .unwrap()
        .iter()
        .find(|r| r.contract == llm_client::Contract::RubricScore)
        .cloned()
        .unwrap();
    let body: serde_json::Value = serde_json::from_str(&call.messages[0].content).unwrap();
    assert_eq!(
        body["response"],
        "Hello, I am Dewi.\nI am from Bandung. Where are you from?"
    );
    assert_eq!(body["level"], Level::A1.as_str());
}

#[tokio::test]
async fn a_scored_roleplay_without_its_rubric_is_stored_unscored() {
    let llm = ReactiveLlm::new();
    let f = fixture_with(llm.clone(), None, RubricCatalog::default()).await;
    let mut unit = f.unit.clone();
    for activity in &mut unit.activities {
        if let Activity::Roleplay(a) = activity {
            a.scoring = Scoring::Rubric;
        }
    }
    let mut p = player_of(&f, unit).await;
    let result = play_roleplay(&mut p).await;
    assert!(matches!(
        result.outcome,
        ResultOutcome::Unscored(UnscoredReason::InteractionRubricMissing)
    ));
    assert_eq!(llm.rubric_calls(), 0);
}

/// A mediation activity built on the example unit's rubric catalog. Written as a
/// test: the example unit has no mediation (E16 wants one every third B2 unit).
fn with_mediation(unit: &Unit, spoken: bool) -> Unit {
    let mut unit = unit.clone();
    unit.activities
        .push(Activity::Mediation(curriculum::Mediation {
            id: "m1-relay".to_owned(),
            skill: curriculum::Skill::Mediation,
            objective_ids: vec!["o6-write-intro".to_owned()],
            instructions: curriculum::Localized {
                en: "Relay the message.".to_owned(),
                id: None,
            },
            scoring: Scoring::Rubric,
            source_text: "The class starts at eight in the morning.".to_owned(),
            task: curriculum::Localized {
                en: "Tell your friend when the class starts.".to_owned(),
                id: None,
            },
            rubric_id: if spoken {
                "rubric-a1-spoken-production".to_owned()
            } else {
                "rubric-a1-written-production".to_owned()
            },
            model_answers: Vec::new(),
        }));
    unit
}

#[tokio::test]
async fn a_mediation_runs_end_to_end_and_files_under_the_channel_it_was_answered_in() {
    // Written: text in, written-production rubric, writing evidence.
    let llm = ReactiveLlm::new();
    let f = fixture(&llm, None).await;
    let mut p = player_of(&f, with_mediation(&f.unit, false)).await;
    let written = p
        .submit("m1-relay", text("The class starts at eight."), &cancel())
        .await
        .unwrap();
    assert_eq!(written.skill, "writing");
    assert_eq!(written.activity_type, curriculum::ActivityType::Mediation);
    let ResultOutcome::Rubric(outcome) = &written.outcome else {
        panic!("scored: {:?}", written.outcome);
    };
    assert_eq!(outcome.dimensions.len(), 4, "a production rubric");
    assert_eq!(written.score, Some(0.75));
    // The scorer saw the source text with the task, so it can judge the relay.
    let call = llm
        .structured_seen
        .lock()
        .unwrap()
        .iter()
        .find(|r| r.contract == llm_client::Contract::RubricScore)
        .cloned()
        .unwrap();
    let body: serde_json::Value = serde_json::from_str(&call.messages[0].content).unwrap();
    assert!(
        body["task_prompt"]
            .as_str()
            .unwrap()
            .contains("The class starts at eight in the morning."),
        "{body}"
    );
    let rows = f.db.attempts().for_session(p.session_id()).await.unwrap();
    assert!(rows.iter().all(|a| a.activity_type == "mediation"));
    assert!(rows.iter().all(|a| a.skill == "writing"));
    assert!(rows.iter().all(|a| a.counts_toward_estimate));

    // Spoken: a transcript in, spoken-production rubric, speaking evidence.
    let llm = ReactiveLlm::new();
    let f = fixture(&llm, None).await;
    let mut p = player_of(&f, with_mediation(&f.unit, true)).await;
    let said = p
        .submit(
            "m1-relay",
            spoken("The class starts at eight in the morning."),
            &cancel(),
        )
        .await
        .unwrap();
    assert_eq!(said.skill, "speaking");
    assert_eq!(said.score, Some(0.75));
    let rows = f.db.attempts().for_session(p.session_id()).await.unwrap();
    assert!(rows.iter().all(|a| a.skill == "speaking"));
    // A spoken response is analysed as voice, so the prompt tells the model to
    // ignore the transcript's spelling.
    let call = llm
        .structured_seen
        .lock()
        .unwrap()
        .iter()
        .find(|r| r.contract == llm_client::Contract::RubricScore)
        .cloned()
        .unwrap();
    let body: serde_json::Value = serde_json::from_str(&call.messages[0].content).unwrap();
    assert_eq!(body["input_mode"], "voice");
}

#[tokio::test]
async fn no_attempt_of_a_unit_run_is_generated_or_free_mode() {
    let llm = ReactiveLlm::new();
    let drill = FakeDrill::new(DrillMode::Score(0.7));
    let f = fixture(&llm, Some(drill)).await;
    let mut p = player(&f).await;
    answer_all_but_roleplay(&mut p).await;
    play_roleplay(&mut p).await;
    let all = f.db.attempts().for_session(p.session_id()).await.unwrap();
    assert!(all.iter().all(|a| a.origin == AttemptOrigin::Authored));
    // The sha of the file is what the index holds, so evidence can be matched to content.
    let unit = f.db.curriculum().unit("a1-u01").await.unwrap().unwrap();
    assert_eq!(unit.file_sha256, f.sha);
}
