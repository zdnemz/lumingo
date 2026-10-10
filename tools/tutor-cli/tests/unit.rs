#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! `tutor-cli unit`: a full scripted completion of the example unit with a fake
//! provider (test code only), the typed-input path, and the program as a user
//! runs it, offline. No hardware, no network, no real provider.

use std::io::{BufRead, BufReader as IoBufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use llm_client::{
    Capabilities, Contract, FinishReason, LadderLevel, LlmClient, LlmError, StreamEvent,
    StreamSummary, StructuredOutput, StructuredRequest, TextRequest, TextStream,
};
use serde_json::{Value, json};
use storage::{AttemptStatus, Database, UnitStatus};
use tokio::io::BufReader;
use tokio_util::sync::CancellationToken;
use tutor_cli::unit::{play_unit, result_json};
use tutor_cli::unit_script;
use tutor_cli::unit_source::Source;
use tutor_engine::{RubricCatalog, UnitConfig, UnitEnv, UnitPlayer, ensure_indexed, system_clock};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn unit_path() -> PathBuf {
    root().join("curriculum/examples/a1-u01.example.json")
}

fn script_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/a1-u01-responses.json")
}

/// A provider that answers by looking at the request: a band of 3 in every rubric
/// dimension with a quote from the response, an empty analysis of the turns it is
/// given, and a fixed tutor line. Test code only.
struct FakeProvider {
    seen: Mutex<Vec<Contract>>,
}

impl FakeProvider {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            seen: Mutex::new(Vec::new()),
        })
    }

    fn calls(&self, contract: Contract) -> usize {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|c| **c == contract)
            .count()
    }
}

#[async_trait]
impl LlmClient for FakeProvider {
    async fn stream_text(
        &self,
        _request: TextRequest,
        _cancel: CancellationToken,
    ) -> Result<TextStream, LlmError> {
        let events: Vec<Result<StreamEvent, LlmError>> = vec![
            Ok(StreamEvent::Delta(
                "Nice to meet you. Where are you from?".to_owned(),
            )),
            Ok(StreamEvent::Finished(StreamSummary {
                finish: FinishReason::Stop,
                usage: None,
                time_to_first_token: Some(Duration::from_millis(1)),
            })),
        ];
        Ok(TextStream::new(futures_util::stream::iter(events)))
    }

    async fn structured(
        &self,
        request: StructuredRequest,
        _cancel: CancellationToken,
    ) -> Result<StructuredOutput, LlmError> {
        self.seen.lock().unwrap().push(request.contract);
        let body: Value = serde_json::from_str(&request.messages[0].content).unwrap();
        let value = match request.contract {
            Contract::RubricScore => {
                let response = body["response"].as_str().unwrap_or_default();
                let quote = response
                    .split_whitespace()
                    .take(3)
                    .collect::<Vec<_>>()
                    .join(" ");
                json!({
                    "dimension_scores": body["rubric"].as_array().unwrap().iter().map(|d| json!({
                        "dimension": d["dimension"], "band": "3",
                        "evidence_quotes": [quote], "reason": "A short reason."
                    })).collect::<Vec<_>>(),
                    "content_points": body["content_points"].as_array().unwrap().iter()
                        .map(|p| json!({ "point": p, "covered": true, "quote": quote }))
                        .collect::<Vec<_>>(),
                    "on_task": true,
                    "feedback_en": "You did the task. Add one more detail.",
                    "feedback_l1": "Kamu menyelesaikan tugasnya. Tambahkan satu detail lagi."
                })
            }
            Contract::TurnAnalysis => json!({ "turns": body["turns"].as_array().unwrap().iter()
                .map(|t| json!({
                    "turn_seq": t["turn_seq"], "errors": [], "objective_evidence": [],
                    "understood_tutor": "yes", "note_for_next_turn": ""
                })).collect::<Vec<_>>() }),
            _ => return Err(LlmError::Protocol("not made by this fake".into())),
        };
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

struct Run {
    _dir: tempfile::TempDir,
    db: Database,
    player: UnitPlayer,
    unit: curriculum::Unit,
}

async fn start(client: Arc<dyn LlmClient>) -> Run {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path().join("unit.sqlite"))
        .await
        .unwrap();
    let profile = db
        .profiles()
        .create(&storage::NewProfile {
            display_name: "Learner".to_owned(),
            ui_language: storage::UiLanguage::Id,
            l1: "id".to_owned(),
            l1_help_mode: storage::L1HelpMode::Auto,
            created_at: storage::Timestamp::now(),
        })
        .await
        .unwrap();
    let loaded = curriculum::load_unit_file(&unit_path()).unwrap();
    ensure_indexed(
        &db,
        &loaded.unit,
        &loaded.checksum,
        storage::Timestamp::now(),
    )
    .await
    .unwrap();
    let rubrics = RubricCatalog::load_dir(&root().join("curriculum/catalogs/rubrics")).unwrap();
    let env = UnitEnv {
        client,
        db: db.clone(),
        clock: system_clock(),
        model: "fake-model".into(),
        provider_profile_id: None,
        provider_qualified: true,
        grammar: None,
        word_levels: None,
        rubrics: Arc::new(rubrics),
        drill: None,
        tts: None,
    };
    let unit = loaded.unit.clone();
    let player = UnitPlayer::start(
        env,
        UnitConfig {
            profile_id: profile.id,
            first_language: "Indonesian".into(),
            app_version: "0.0.0-test".into(),
        },
        loaded.unit,
    )
    .await
    .unwrap();
    Run {
        _dir: dir,
        db,
        player,
        unit,
    }
}

type Empty = BufReader<tokio::io::Empty>;

fn script_source() -> Source<Empty> {
    let file = unit_script::load(&script_path()).unwrap();
    Source::script(file, script_path().parent().unwrap())
}

fn assert_no_level_is_named(text: &str) {
    for token in text.split(|c: char| !c.is_ascii_alphanumeric()) {
        assert!(
            !matches!(token, "A1" | "A2" | "B1" | "B2" | "C1" | "C2"),
            "the output names a level: {token}"
        );
    }
    assert!(!text.to_lowercase().contains("cefr"));
}

#[tokio::test]
async fn the_example_unit_is_completed_from_a_script_with_its_reading_listening_and_writing_tasks()
{
    let provider = FakeProvider::new();
    let mut run = start(provider.clone()).await;
    let mut source = script_source();
    let mut out = Vec::new();
    let report = play_unit(
        &mut run.player,
        &mut source,
        &mut out,
        true,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let text = String::from_utf8(out).unwrap();

    // Every activity but the read-aloud (no recording in the sample script) was answered.
    assert_eq!(report.skipped, ["a07-read-aloud-thanks"]);
    assert_eq!(report.results.len(), 16);
    let summary = report.summary.as_ref().expect("the checkpoint is decided");
    assert!(summary.checkpoint.outcome.passed);
    assert!(!summary.checkpoint.outcome.provisional);
    assert_eq!(summary.status, UnitStatus::Passed);

    // Per-activity scores are printed.
    for needle in [
        "== a14-read-set-class-chat (reading_set) ==",
        "a14-read-set-class-chat  [reading]  score 1.00",
        "== a15-listen-set-putu (listening_set) ==",
        "a15-listen-set-putu  [listening]  score 1.00",
        "[audio played: 2 time(s), replays left: 1]",
        "a10-speak-introduce  [speaking]  score 0.75 (confidence 0.90)",
        "a12-write-introduce  [writing]  score 0.75 (confidence 0.90)",
        "a16-fix-missing-am  [writing]  score 1.00",
        "a11-roleplay-classmate  [speaking]  not scored",
        "tutor> Nice to meet you. Where are you from?",
        "you> Hello, I am Dewi.",
        "(skipped: no response)",
        "unit status: Passed",
    ] {
        assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
    }
    assert!(text.contains("checkpoint\n"));
    assert!(text.contains("against a pass mark of 0.70: passed"));
    assert_no_level_is_named(&text);

    // The rows are in the database: 12 objective items, two rubric tasks of four
    // dimensions each, and the two practice items that have no scorer.
    let attempts = run
        .db
        .attempts()
        .for_session(report.session_id)
        .await
        .unwrap();
    assert_eq!(attempts.len(), 12 + 8 + 2);
    for id in [
        "a14-read-set-class-chat",
        "a15-listen-set-putu",
        "a10-speak-introduce",
        "a12-write-introduce",
        "a16-fix-missing-am",
        "a17-fix-missing-from",
    ] {
        assert!(attempts.iter().any(|a| a.activity_id == id), "{id}");
    }
    assert!(
        attempts
            .iter()
            .filter(|a| a.status == AttemptStatus::Scored)
            .all(|a| a.counts_toward_estimate)
    );
    // Two runs for each productive checkpoint item.
    assert_eq!(provider.calls(Contract::RubricScore), 4);

    // The result file carries scores and statuses and no level.
    let result = result_json(&run.unit.id, &report, None);
    let json_text = serde_json::to_string(&result).unwrap();
    assert_no_level_is_named(&json_text);
    assert_eq!(result["unit"], "a1-u01");
    assert_eq!(result["checkpoint"]["passed"], true);
    let activities = result["activities"].as_array().unwrap();
    let status_of = |id: &str| {
        activities
            .iter()
            .find(|a| a["id"] == id)
            .map(|a| a["status"].as_str().unwrap().to_owned())
    };
    assert_eq!(status_of("a10-speak-introduce").as_deref(), Some("scored"));
    assert_eq!(
        status_of("a11-roleplay-classmate").as_deref(),
        Some("unscored")
    );
    assert_eq!(
        status_of("a07-read-aloud-thanks").as_deref(),
        Some("skipped")
    );
}

#[tokio::test]
async fn a_missing_response_for_a_checkpoint_item_leaves_the_checkpoint_undecided() {
    let mut run = start(FakeProvider::new()).await;
    let mut file = unit_script::load(&script_path()).unwrap();
    file.responses.remove("a12-write-introduce");
    let mut source: Source<Empty> = Source::script(file, script_path().parent().unwrap());
    let mut out = Vec::new();
    let report = play_unit(
        &mut run.player,
        &mut source,
        &mut out,
        true,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(report.summary.is_none());
    assert!(
        text.contains("checkpoint not decided") && text.contains("a12-write-introduce"),
        "{text}"
    );
    let session = run
        .db
        .sessions()
        .get(report.session_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(session.status, storage::SessionStatus::Aborted);
    let progress = run
        .db
        .unit_progress()
        .get(1, "a1-u01")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(progress.status, UnitStatus::InProgress);
}

#[tokio::test]
async fn a_response_the_activity_cannot_take_is_refused_by_name_and_the_run_goes_on() {
    let mut run = start(FakeProvider::new()).await;
    let mut file = unit_script::load(&script_path()).unwrap();
    // Two gaps wanted, one given.
    file.responses.get_mut("a03-gap-am").unwrap().gaps = Some(vec!["am".to_owned()]);
    let mut source: Source<Empty> = Source::script(file, script_path().parent().unwrap());
    let mut out = Vec::new();
    let report = play_unit(
        &mut run.player,
        &mut source,
        &mut out,
        true,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains("(refused: the response has 1 entries and the activity has 2)"),
        "{text}"
    );
    assert!(report.skipped.contains(&"a03-gap-am".to_owned()));
    // a03 is in the checkpoint, so it is undecided: nothing was stored for it.
    assert!(report.summary.is_none());
    let attempts = run
        .db
        .attempts()
        .for_session(report.session_id)
        .await
        .unwrap();
    assert!(attempts.iter().all(|a| a.activity_id != "a03-gap-am"));
}

#[tokio::test]
async fn a_run_with_typed_answers_asks_again_after_a_bad_line_and_skips_on_request() {
    let mut run = start(FakeProvider::new()).await;
    let typed = "\
x
1
2
am | from
my name is Dewi
1 2 3 4
Where are you from?

1 2 1

Hello I am Dewi and I am from Bandung
Hello, I am Dewi.
/end
Hello I am Dewi. I am from Bandung. Nice to meet you.
2
2 3 3 1
1 2 2
I am Dewi.
/skip
";
    let mut source = Source::typed(BufReader::new(typed.as_bytes()));
    let mut out = Vec::new();
    let report = play_unit(
        &mut run.player,
        &mut source,
        &mut out,
        false,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("\"x\" is not a number"), "{text}");
    assert!(
        text.contains("[audio: 1 line(s), play it with the speaker]"),
        "audio text is hidden on request"
    );
    assert!(!text.contains("Good morning! I'm Dewi. What's your name?"));
    // The read-aloud has a scorer and was given a completion, which it cannot take.
    assert!(
        text.contains("(refused: the response does not fit this activity"),
        "{text}"
    );
    assert!(report.skipped.contains(&"a07-read-aloud-thanks".to_owned()));
    assert!(report.skipped.contains(&"a17-fix-missing-from".to_owned()));
    let summary = report.summary.expect("the checkpoint is decided");
    assert!(summary.checkpoint.outcome.passed);
    // A typed answer to the speaking task is stored as its transcript, in voice mode.
    let speaking = report
        .results
        .iter()
        .find(|r| r.activity_id == "a10-speak-introduce")
        .unwrap();
    assert_eq!(speaking.skill, "speaking");
    assert_eq!(speaking.score, Some(0.75));
}

fn tutor_cli(args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_tutor-cli"));
    command
        .args(args)
        .env_remove("RUST_LOG")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for name in [
        "TUTOR_LLM_PROTOCOL",
        "TUTOR_LLM_BASE_URL",
        "TUTOR_LLM_MODEL",
        "TUTOR_LLM_API_KEY",
    ] {
        command.env_remove(name);
    }
    command
}

#[tokio::test]
async fn offline_the_program_stores_the_attempts_waits_on_the_provider_and_writes_the_result_file()
{
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("run.sqlite");
    let out_path = dir.path().join("result.json");
    let unit = unit_path();
    let script = script_path();
    let output = tutor_cli(&[
        "unit",
        "run",
        unit.to_str().unwrap(),
        "--script",
        script.to_str().unwrap(),
        "--offline",
        "--db",
        db_path.to_str().unwrap(),
        "--out",
        out_path.to_str().unwrap(),
    ])
    .current_dir(dir.path())
    .output()
    .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(output.status.code(), Some(0), "{stdout}\n{stderr}");
    for needle in [
        "unit a1-u01: Hello! Nice to meet you",
        "provider: none: productive responses will be stored as pending",
        "a10-speak-introduce  [speaking]  waiting for a provider, stored as pending",
        "a12-write-introduce  [writing]  waiting for a provider, stored as pending",
        "(the provider is not available: the roleplay cannot run)",
        "passed provisionally: some items still wait for a score",
        "unit status: InProgress",
        "2 response(s) wait for a provider",
        "tutor-cli unit score-pending --db",
    ] {
        assert!(stdout.contains(needle), "missing {needle:?} in:\n{stdout}");
    }
    assert_no_level_is_named(&stdout);

    let result: Value = serde_json::from_str(&std::fs::read_to_string(&out_path).unwrap()).unwrap();
    assert_eq!(result["checkpoint"]["provisional"], true);
    assert_eq!(result["checkpoint"]["passed"], true);
    let status_of = |id: &str| {
        result["activities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["id"] == id)
            .map(|a| a["status"].as_str().unwrap().to_owned())
    };
    assert_eq!(status_of("a10-speak-introduce").as_deref(), Some("pending"));
    assert_eq!(status_of("a02-greeting-by-time").as_deref(), Some("scored"));

    // The database holds pending rows and a queue, and no estimate or level.
    let db = Database::open(&db_path).await.unwrap();
    let pending = db.pending_scoring().oldest(10).await.unwrap();
    assert_eq!(pending.len(), 2);
    let attempts = db
        .attempts()
        .for_session(result["session_id"].as_i64().unwrap())
        .await
        .unwrap();
    assert_eq!(
        attempts
            .iter()
            .filter(|a| a.status == AttemptStatus::PendingLlm)
            .count(),
        8
    );
    assert!(db.estimates().latest_per_skill(1).await.unwrap().is_empty());

    // With no provider, scoring the backlog says so and exits with the provider code.
    let pending_run = tutor_cli(&[
        "unit",
        "score-pending",
        "--db",
        db_path.to_str().unwrap(),
        "--data-dir",
        dir.path().to_str().unwrap(),
    ])
    .current_dir(dir.path())
    .output()
    .unwrap();
    assert_eq!(pending_run.status.code(), Some(3));
    let stderr = String::from_utf8(pending_run.stderr).unwrap();
    assert!(stderr.contains("no provider is configured"), "{stderr}");
    assert_eq!(
        db.pending_scoring().oldest(10).await.unwrap().len(),
        2,
        "left as they were"
    );
}

#[test]
fn the_program_needs_a_script_or_interactive_and_refuses_a_bad_script() {
    let none = tutor_cli(&["unit", "run", unit_path().to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(none.status.code(), Some(2), "a usage error");
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("bad.json");
    std::fs::write(&bad, r#"{"responses": {"a01": {"chioce": 1}}}"#).unwrap();
    let output = tutor_cli(&[
        "unit",
        "run",
        unit_path().to_str().unwrap(),
        "--script",
        bad.to_str().unwrap(),
        "--offline",
        "--db",
        dir.path().join("x.sqlite").to_str().unwrap(),
    ])
    .output()
    .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("not a valid responses file")
    );
    let missing = tutor_cli(&[
        "unit",
        "run",
        "no-such-unit.json",
        "--script",
        bad.to_str().unwrap(),
        "--offline",
    ])
    .output()
    .unwrap();
    assert_eq!(missing.status.code(), Some(1));
}

/// A provider on 127.0.0.1 that answers the probe and the rubric contract, so
/// the binary's `unit score-pending` can drain a queue. Test code only.
struct LocalProvider {
    base_url: String,
}

impl LocalProvider {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                std::thread::spawn(move || serve(stream));
            }
        });
        Self {
            base_url: format!("http://127.0.0.1:{port}/v1"),
        }
    }
}

fn serve(stream: TcpStream) {
    let mut reader = IoBufReader::new(stream);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
        return;
    }
    let mut length = 0_usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
            break;
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0_u8; length];
    let _ = reader.read_exact(&mut body);
    let request: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let (content_type, payload) = if request["stream"] == true {
        // The probe's streaming step: one short sentence, then the end.
        let chunk = |delta: Value, finish: Value| {
            format!(
                "data: {{\"id\":\"c1\",\"object\":\"chat.completion.chunk\",\"model\":\"m\",\"choices\":[{{\"index\":0,\"delta\":{delta},\"finish_reason\":{finish}}}]}}\n\n"
            )
        };
        let payload = format!(
            "{}{}{}data: [DONE]\n\n",
            chunk(json!({"role": "assistant", "content": ""}), Value::Null),
            chunk(json!({"content": "It is sunny."}), Value::Null),
            chunk(json!({}), json!("stop")),
        );
        ("text/event-stream", payload)
    } else {
        let schema_name = request["response_format"]["json_schema"]["name"]
            .as_str()
            .unwrap_or_default();
        let content = match schema_name {
            // The probe's step 3 asks for the canary field; a provider that
            // answers it passes level 1, which the probe then caches.
            "probe_test" => {
                r#"{"title":"A short test","word_count":3,"is_ok":true,"check":"schema_received"}"#
                    .to_owned()
            }
            // The drain: a band of 3 in every dimension of the rubric that was
            // sent, with a quote taken from the response (X1 accepts it).
            "rubric_score" => {
                let response = request["messages"]
                    .as_array()
                    .and_then(|messages| messages.last())
                    .and_then(|message| message["content"].as_str())
                    .unwrap_or_default();
                let body: Value = serde_json::from_str(response).unwrap_or(Value::Null);
                let quote = body["response"]
                    .as_str()
                    .unwrap_or_default()
                    .split_whitespace()
                    .take(3)
                    .collect::<Vec<_>>()
                    .join(" ");
                let dimensions: Vec<Value> = body["rubric"]
                    .as_array()
                    .map(|dimensions| {
                        dimensions
                            .iter()
                            .map(|dimension| {
                                json!({
                                    "dimension": dimension["dimension"],
                                    "band": "3",
                                    "evidence_quotes": [quote],
                                    "reason": "A short reason."
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let points: Vec<Value> = body["content_points"]
                    .as_array()
                    .map(|points| {
                        points
                            .iter()
                            .map(|point| json!({ "point": point, "covered": true, "quote": quote }))
                            .collect()
                    })
                    .unwrap_or_default();
                json!({
                    "dimension_scores": dimensions,
                    "content_points": points,
                    "on_task": true,
                    "feedback_en": "You did the task. Add one more detail.",
                    "feedback_l1": "Kamu menyelesaikan tugasnya. Tambahkan satu detail lagi."
                })
                .to_string()
            }
            // Every other contract: an empty object, which the ladder rejects
            // as a support failure and the probe moves past.
            _ => "{}".to_owned(),
        };
        let payload = json!({
            "id": "chatcmpl-test",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": content, "refusal": null },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15 }
        })
        .to_string();
        ("application/json", payload)
    };
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{payload}",
        payload.len()
    );
    let mut stream = reader.into_inner();
    let _ = stream.write_all(response.as_bytes());
}

#[test]
fn a_backlog_drain_writes_the_estimates() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("drain.sqlite");
    // A run with no provider leaves the two productive responses queued.
    let offline = tutor_cli(&[
        "unit",
        "run",
        unit_path().to_str().unwrap(),
        "--script",
        script_path().to_str().unwrap(),
        "--offline",
        "--db",
        db_path.to_str().unwrap(),
    ])
    .current_dir(dir.path())
    .output()
    .unwrap();
    let stdout = String::from_utf8(offline.stdout).unwrap();
    assert_eq!(offline.status.code(), Some(0), "{stdout}");

    // With a provider, the drain scores them and the estimates follow.
    let provider = LocalProvider::start();
    let mut command = tutor_cli(&[
        "unit",
        "score-pending",
        "--db",
        db_path.to_str().unwrap(),
        "--data-dir",
        dir.path().to_str().unwrap(),
    ]);
    command
        .env("TUTOR_LLM_PROTOCOL", "openai_chat")
        .env("TUTOR_LLM_BASE_URL", &provider.base_url)
        .env("TUTOR_LLM_MODEL", "fake-model")
        .env("TUTOR_LLM_API_KEY", "sk-test-0123456789abcdef");
    let drained = command.current_dir(dir.path()).output().unwrap();
    let stdout = String::from_utf8(drained.stdout).unwrap();
    let stderr = String::from_utf8(drained.stderr).unwrap();
    assert_eq!(drained.status.code(), Some(0), "{stdout}\n{stderr}");
    assert!(stdout.contains("scored 2,"), "{stdout}");

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let db = Database::open(&db_path).await.unwrap();
        let estimates = db.estimates().latest_per_skill(1).await.unwrap();
        assert_eq!(
            estimates.len(),
            4,
            "the drain recomputes every skill: {estimates:?}"
        );
        assert!(
            estimates.iter().all(|row| row.algorithm_version == "est/1"),
            "{estimates:?}"
        );
        // The drained responses are evidence: a12's four dimension rows are one
        // observation, and the two error-correction items are one each.
        let speaking = estimates
            .iter()
            .find(|row| row.skill == "speaking")
            .expect("a row for speaking");
        assert_eq!(speaking.evidence_count, 1, "{speaking:?}");
        let writing = estimates
            .iter()
            .find(|row| row.skill == "writing")
            .expect("a row for writing");
        assert_eq!(writing.evidence_count, 3, "{writing:?}");
    });
}
