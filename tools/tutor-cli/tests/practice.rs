#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! `tutor-cli unit practice` (S4-10): generated items are converted, validated,
//! marked generated and never counted toward an estimate; the authored fallback
//! replays the wrong answers of the newest lesson run. A fake provider (test
//! code only) stands in for the model; the binary test runs the real program
//! offline.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use llm_client::{
    Capabilities, Contract, LadderLevel, LlmClient, LlmError, StructuredOutput, StructuredRequest,
    TextRequest, TextStream,
};
use serde_json::{Value, json};
use storage::{AttemptOrigin, Database};
use tempfile::TempDir;
use tutor_cli::practice::{PracticeReport, result_json, run};
use tutor_cli::unit::Connection;
use tutor_engine::NoProvider;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn unit_path() -> PathBuf {
    root().join("curriculum/examples/a1-u01.example.json")
}

/// A provider that answers `practice_items` with three valid items and nothing
/// else. Test code only.
struct FakePractice {
    seen: Mutex<Vec<Value>>,
}

impl FakePractice {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            seen: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait]
impl LlmClient for FakePractice {
    async fn stream_text(
        &self,
        _request: TextRequest,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> Result<TextStream, LlmError> {
        Err(LlmError::Protocol("not made by this fake".into()))
    }

    async fn structured(
        &self,
        request: StructuredRequest,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> Result<StructuredOutput, LlmError> {
        let body: Value = serde_json::from_str(&request.messages[0].content).unwrap();
        if request.contract != Contract::PracticeItems {
            return Err(LlmError::Protocol("not made by this fake".into()));
        }
        self.seen.lock().unwrap().push(body);
        let value = json!({ "items": [
            {
                "type": "mcq", "objective_id": "o2-introduce", "grammar_id": "g-be-i-am",
                "stem": "Sari says: I ___ a teacher.", "options": ["am", "is", "are"],
                "answer_index": 0, "text": "", "answers": [], "tokens": [], "answer": "",
                "explanation_en": "Use am with I.", "explanation_l1": "Pakai am dengan I."
            },
            {
                "type": "gap_fill", "objective_id": "o2-introduce", "grammar_id": "g-be-from",
                "stem": "", "options": [], "answer_index": -1,
                "text": "Putu ___ from Bali.", "answers": [["is"]], "tokens": [], "answer": "",
                "explanation_en": "Use is with he or she.", "explanation_l1": "Pakai is dengan dia."
            },
            {
                "type": "reorder", "objective_id": "o2-introduce", "grammar_id": "g-be-from",
                "stem": "", "options": [], "answer_index": -1, "text": "", "answers": [],
                "tokens": ["from", "I", "am", "Bali"], "answer": "I am from Bali",
                "explanation_en": "Say where you are from.", "explanation_l1": "Katakan asalmu."
            }
        ]});
        Ok(StructuredOutput {
            value,
            ladder_level: LadderLevel::NativeSchema,
            repaired: false,
            calls: 1,
            usage: None,
        })
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }
}

fn script_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/a1-u01-responses.json")
}

/// A script that answers three generated items by position and the two
/// authored items the offline replay can offer, right this time.
fn generated_script(dir: &Path) -> PathBuf {
    let path = dir.join("generated-responses.json");
    let body = json!({ "responses": {
        "gen-1": { "choice": 0 },
        "gen-2": { "gaps": ["is"] },
        "gen-3": { "order": ["I", "am", "from", "Bali"] },
        "a03-gap-am": { "gaps": ["am", "from"] },
        "a02-greeting-by-time": { "choice": 1 }
    }});
    std::fs::write(&path, serde_json::to_string_pretty(&body).unwrap()).unwrap();
    path
}

fn args(unit: &Path, db: &Path, script: Option<&Path>) -> tutor_cli::args::PracticeArgs {
    tutor_cli::args::PracticeArgs {
        unit: unit.to_owned(),
        count: 3,
        script: script.map(Path::to_owned),
        interactive: false,
        word_list: None,
        db: Some(db.to_owned()),
        out: None,
        first_language: "Indonesian".to_owned(),
        offline: false,
        provider: None,
        providers_file: None,
        data_dir: None,
        provider_timeout_ms: 16_000,
    }
}

fn connection(client: Arc<dyn LlmClient>, model: &str) -> Connection {
    Connection {
        client,
        model: model.to_owned(),
        label: format!("fake ({model})"),
    }
}

fn loaded() -> curriculum::LoadedUnit {
    curriculum::load_unit_file(&unit_path()).unwrap()
}

#[tokio::test]
async fn generated_items_are_stored_marked_generated_and_never_count() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("practice.sqlite");
    let script = generated_script(dir.path());
    let provider = FakePractice::new();
    let args = args(&unit_path(), &db_path, Some(&script));
    let exit = run(
        &args,
        &loaded(),
        None,
        connection(provider.clone(), "fake-model"),
    )
    .await
    .unwrap();
    assert_eq!(exit, tutor_cli::exit::Exit::Done);

    let db = Database::open(&db_path).await.unwrap();
    let sessions = db.sessions().list_for_profile(1, 10).await.unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].kind, storage::SessionKind::Drill);
    assert_eq!(sessions[0].status, storage::SessionStatus::Completed);

    let attempts = db.attempts().for_session(sessions[0].id).await.unwrap();
    assert_eq!(attempts.len(), 3, "one row per item");
    assert!(
        attempts
            .iter()
            .all(|a| a.origin == AttemptOrigin::Generated)
    );
    assert!(attempts.iter().all(|a| !a.counts_toward_estimate));
    assert!(attempts.iter().all(|a| a.normalized == Some(1.0)));
    assert_eq!(
        attempts
            .iter()
            .map(|a| a.activity_id.as_str())
            .collect::<Vec<_>>(),
        [
            format!("gen-s{}-1", sessions[0].id),
            format!("gen-s{}-2", sessions[0].id),
            format!("gen-s{}-3", sessions[0].id),
        ]
    );

    // The items are stored with the session, as the contract asks.
    let stored = db
        .generated_content()
        .for_session(sessions[0].id)
        .await
        .unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].kind, storage::GeneratedKind::PracticeItems);
    assert_eq!(stored[0].contract_version, "practice_items/1");

    // Nothing wrote an estimate.
    assert!(db.estimates().latest_per_skill(1).await.unwrap().is_empty());

    // The prompt asked for the unit's own items so they are not repeated.
    let seen = provider.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert!(
        !seen[0]["existing_items"].as_array().unwrap().is_empty(),
        "the authored items are sent as existing"
    );
}

#[tokio::test]
async fn offline_the_wrong_answers_of_the_newest_lesson_come_first() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("practice.sqlite");
    let db = Database::open(&db_path).await.unwrap();
    db.profiles()
        .create(&storage::NewProfile {
            display_name: "Learner".to_owned(),
            ui_language: storage::UiLanguage::Id,
            l1: "id".to_owned(),
            l1_help_mode: storage::L1HelpMode::Auto,
            created_at: storage::Timestamp::now(),
        })
        .await
        .unwrap();
    // A session may only reference an indexed unit.
    let unit = loaded();
    tutor_engine::ensure_indexed(&db, &unit.unit, &unit.checksum, storage::Timestamp::now())
        .await
        .unwrap();
    // A lesson run with one wrong answer and one right one.
    let lesson = db
        .sessions()
        .create(&storage::NewSession {
            profile_id: 1,
            kind: storage::SessionKind::Lesson,
            unit_id: Some("a1-u01".to_owned()),
            activity_id: None,
            mode: None,
            provider_profile_id: None,
            app_version: "test".to_owned(),
            started_at: storage::Timestamp::now(),
        })
        .await
        .unwrap();
    for (activity, normalized) in [("a03-gap-am", 0.0), ("a02-greeting-by-time", 1.0)] {
        db.attempts()
            .insert(&storage::NewAttempt {
                profile_id: 1,
                session_id: Some(lesson.id),
                unit_id: Some("a1-u01".to_owned()),
                activity_id: activity.to_owned(),
                activity_type: "gap_fill".to_owned(),
                response_id: format!("{activity}@1#0"),
                origin: AttemptOrigin::Authored,
                level: storage::Level::A1,
                skill: "grammar".to_owned(),
                dimension: "grammar".to_owned(),
                scorer: storage::Scorer::Deterministic,
                scorer_version: "gap_fill/1".to_owned(),
                raw_score: Some(normalized),
                max_score: Some(1.0),
                normalized: Some(normalized),
                confidence: Some(1.0),
                status: storage::AttemptStatus::Scored,
                counts_toward_estimate: true,
                created_at: storage::Timestamp::now(),
            })
            .await
            .unwrap();
    }
    db.sessions()
        .finish(
            lesson.id,
            storage::SessionStatus::Completed,
            &storage::Timestamp::now(),
            None,
        )
        .await
        .unwrap();

    // Offline: the fallback replays the wrong answer first.
    let mut args = args(&unit_path(), &db_path, Some(&script_path()));
    args.offline = true;
    args.count = 2;
    let exit = run(
        &args,
        &loaded(),
        None,
        connection(Arc::new(NoProvider), "none"),
    )
    .await
    .unwrap();
    assert_eq!(exit, tutor_cli::exit::Exit::Done);

    let db = Database::open(&db_path).await.unwrap();
    let practice = db
        .sessions()
        .list_for_profile(1, 10)
        .await
        .unwrap()
        .into_iter()
        .find(|s| s.kind == storage::SessionKind::Drill)
        .unwrap();
    let attempts = db.attempts().for_session(practice.id).await.unwrap();
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0].activity_id, "a03-gap-am", "the wrong one first");
    assert!(
        attempts
            .iter()
            .all(|a| a.origin == AttemptOrigin::Authored && !a.counts_toward_estimate),
        "a replay keeps its authored origin and still does not count"
    );
}

#[tokio::test]
async fn the_result_file_names_the_source_and_carries_no_level() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("practice.sqlite");
    let script = generated_script(dir.path());
    let provider = FakePractice::new();
    let args = args(&unit_path(), &db_path, Some(&script));
    run(&args, &loaded(), None, connection(provider, "fake-model"))
        .await
        .unwrap();

    // Read the database back the way the result file does.
    let db = Database::open(&db_path).await.unwrap();
    let session = db
        .sessions()
        .list_for_profile(1, 10)
        .await
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let attempts = db.attempts().for_session(session.id).await.unwrap();
    let played = tutor_cli::practice::Played {
        items: attempts
            .iter()
            .map(|a| tutor_cli::practice::PlayedItem {
                id: a.activity_id.clone(),
                activity_id: a.activity_id.clone(),
                activity_type: a.activity_type.clone(),
                origin: a.origin,
                score: a.normalized.unwrap_or_default(),
            })
            .collect(),
        skipped: Vec::new(),
    };
    let set = tutor_engine::PracticeSet {
        items: Vec::new(),
        source: tutor_engine::PracticeSource::Generated,
        rejected: Vec::new(),
        regenerated: false,
        fallback: None,
        vocabulary_checked: false,
    };
    let report = PracticeReport {
        session_id: session.id,
        set: &set,
        played,
    };
    let value = result_json("a1-u01", &report, None);
    let text = serde_json::to_string(&value).unwrap();
    assert!(!text.contains("A1") && !text.to_lowercase().contains("cefr"));
    assert_eq!(value["source"], "generated");
    assert_eq!(value["items"].as_array().unwrap().len(), 3);
    assert_eq!(value["items"][0]["origin"], "generated");
}

#[test]
fn the_binary_runs_the_offline_fallback_and_writes_the_result_file() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("run.sqlite");
    let out_path = dir.path().join("result.json");
    let output = Command::new(env!("CARGO_BIN_EXE_tutor-cli"))
        .args([
            "unit",
            "practice",
            unit_path().to_str().unwrap(),
            "--script",
            script_path().to_str().unwrap(),
            "--offline",
            "--db",
            db_path.to_str().unwrap(),
            "--out",
            out_path.to_str().unwrap(),
        ])
        .env_remove("RUST_LOG")
        .env_remove("TUTOR_LLM_API_KEY")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(output.status.code(), Some(0), "{stdout}\n{stderr}");
    assert!(stdout.contains("practice for unit a1-u01"), "{stdout}");
    assert!(
        stdout.contains("none: the authored fallback will be replayed"),
        "{stdout}"
    );
    assert!(
        stdout.contains("set: authored fallback (the provider could not be used), 3 item(s)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("every answer here is practice: none of it counts toward a level estimate"),
        "{stdout}"
    );

    let result: Value = serde_json::from_str(&std::fs::read_to_string(&out_path).unwrap()).unwrap();
    assert_eq!(result["unit"], "a1-u01");
    assert_eq!(result["source"], "authored");
    assert_eq!(result["fallback"], "provider_unavailable");
    assert_eq!(result["items"].as_array().unwrap().len(), 3);

    // The rows are practice: authored replays that do not count.
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let db = Database::open(&db_path).await.unwrap();
        let session = db
            .sessions()
            .list_for_profile(1, 10)
            .await
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        assert_eq!(session.kind, storage::SessionKind::Drill);
        let attempts = db.attempts().for_session(session.id).await.unwrap();
        assert_eq!(attempts.len(), 3);
        assert!(attempts.iter().all(|a| !a.counts_toward_estimate));
        assert!(
            db.estimates().latest_per_skill(1).await.unwrap().is_empty(),
            "no estimate may come from practice"
        );
    });
}
