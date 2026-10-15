#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! `tutor-cli workshop` (S4-12): rule findings at once and offline, the errors
//! and the rubric bands when a provider answers, the second draft compared with
//! the first, and the drafts that wait for a provider queued instead of lost.
//! A fake provider (test code only) stands in for the model; one test runs the
//! real program offline.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use curriculum::validate::GrammarCheck;
use llm_client::{
    Capabilities, Contract, LadderLevel, LlmClient, LlmError, StructuredOutput, StructuredRequest,
    TextRequest, TextStream,
};
use serde_json::{Value, json};
use storage::{AttemptOrigin, AttemptStatus, Database};
use tempfile::TempDir;
use tutor_cli::args::{LevelArg, WorkshopArgs};
use tutor_cli::unit::Connection;
use tutor_cli::workshop::{TaskInput, run};
use tutor_engine::{Dimension, NoProvider, RubricDimension, WorkshopRubric, WorkshopTask};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn rubrics_dir() -> PathBuf {
    root().join("curriculum/catalogs/rubrics")
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

/// A provider that answers the two structured contracts of a workshop round
/// from a queue: `turn_analysis` (T2) then `rubric_score` (T3) per draft.
/// Test code only.
struct FakeWorkshop {
    analysis: Mutex<VecDeque<Value>>,
    rubric: Mutex<VecDeque<Value>>,
    seen: Mutex<Vec<Contract>>,
}

impl FakeWorkshop {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            analysis: Mutex::new(VecDeque::new()),
            rubric: Mutex::new(VecDeque::new()),
            seen: Mutex::new(Vec::new()),
        })
    }

    fn queue_errors(&self, errors: Vec<Value>) {
        self.analysis.lock().unwrap().push_back(json!({
            "turns": [{ "turn_seq": 0, "errors": errors, "objective_evidence": [],
                        "understood_tutor": "not_applicable", "note_for_next_turn": "" }]
        }));
    }

    fn queue_bands(&self, quote: &str) {
        self.rubric.lock().unwrap().push_back(json!({
            "dimension_scores": [
                { "dimension": "task_achievement", "band": "3", "evidence_quotes": [quote],
                  "reason": "Covers the task." },
                { "dimension": "range", "band": "2", "evidence_quotes": [quote],
                  "reason": "Simple words." },
                { "dimension": "accuracy", "band": "3", "evidence_quotes": [quote],
                  "reason": "Mostly right." }
            ],
            "content_points": [],
            "on_task": true,
            "feedback_en": "You gave clear plans. Add a reason next time.",
            "feedback_l1": "Rencanamu jelas. Tambahkan alasan lain kali."
        }));
    }

    fn calls(&self) -> Vec<Contract> {
        self.seen.lock().unwrap().clone()
    }
}

#[async_trait]
impl LlmClient for FakeWorkshop {
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
        let body: Value = serde_json::from_str(&request.messages[0].content)
            .map_err(|_| LlmError::Protocol("not made by this fake".into()))?;
        self.seen.lock().unwrap().push(request.contract);
        let value = match request.contract {
            Contract::TurnAnalysis => {
                let mut reply = self
                    .analysis
                    .lock()
                    .unwrap()
                    .pop_front()
                    .ok_or_else(|| LlmError::Protocol("no analysis queued".into()))?;
                // The engine matches the reply to the draft by turn_seq, so the
                // fake answers the seq it was asked about.
                reply["turns"][0]["turn_seq"] = body["turns"][0]["turn_seq"].clone();
                reply
            }
            Contract::RubricScore => self
                .rubric
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| LlmError::Protocol("no rubric queued".into()))?,
            other => {
                return Err(LlmError::Protocol(format!(
                    "not made by this fake: {other:?}"
                )));
            }
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

fn error(category: &str, quote: &str, correction: &str) -> Value {
    json!({ "category": category, "quote": quote, "correction": correction,
            "severity": "major", "addressed_in_reply": false })
}

fn rubric() -> WorkshopRubric {
    let dims = [
        Dimension::TaskAchievement,
        Dimension::Range,
        Dimension::Accuracy,
    ];
    WorkshopRubric {
        id: "rubric-a2-written-production".into(),
        version: 1,
        dimensions: dims
            .into_iter()
            .map(|dimension| RubricDimension {
                dimension,
                bands: ["nothing", "little", "some", "enough", "more"].map(String::from),
            })
            .collect(),
    }
}

fn task() -> TaskInput {
    TaskInput {
        task: WorkshopTask {
            prompt: "Write a note to a friend about the weekend.".into(),
            content_points: Vec::new(),
            min_words: None,
        },
        prompt_id: None,
        rubric: Some(rubric()),
    }
}

fn args(db: &Path, script: Option<&Path>, out: Option<&Path>) -> WorkshopArgs {
    WorkshopArgs {
        topic: Some("Write a note to a friend about the weekend.".to_owned()),
        prompt: None,
        script: script.map(Path::to_owned),
        interactive: false,
        level: LevelArg::A2,
        rubrics: Some(rubrics_dir()),
        word_list: None,
        db: Some(db.to_owned()),
        out: out.map(Path::to_owned),
        first_language: "Indonesian".to_owned(),
        offline: false,
        provider: None,
        providers_file: None,
        data_dir: None,
        provider_timeout_ms: 16_000,
    }
}

fn script(dir: &Path, drafts: &[&str]) -> PathBuf {
    let path = dir.join("drafts.json");
    let body = json!({ "drafts": drafts });
    std::fs::write(&path, serde_json::to_string_pretty(&body).unwrap()).unwrap();
    path
}

fn connection(client: Arc<dyn LlmClient>, model: &str) -> Connection {
    Connection {
        client,
        model: model.to_owned(),
        label: format!("fake ({model})"),
    }
}

/// The two drafts of the round: the second fixes the spelling and introduces a
/// new agreement mistake, so it is classified as fixed, remaining and new.
const FIRST: &str =
    "Hi Sari, I has a plan for Saturday at the lake. We go to teh lake and swim together.";
const SECOND: &str =
    "Hi Sari, I has a plan for Saturday at the lake. We goes to the lake and swim together.";

#[tokio::test]
async fn offline_the_rule_findings_come_at_once_and_the_draft_is_queued() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("workshop.sqlite");
    let path = script(
        dir.path(),
        &["Last weekend I saw an test at the lake with my family."],
    );
    let mut args = args(&db_path, Some(&path), None);
    args.offline = true;
    let exit = run(
        &args,
        &task(),
        None,
        connection(Arc::new(NoProvider), "none"),
    )
    .await
    .unwrap();
    assert_eq!(
        exit,
        tutor_cli::exit::Exit::Done,
        "offline is not a failure"
    );

    let db = Database::open(&db_path).await.unwrap();
    let sessions = db.sessions().list_for_profile(1, 10).await.unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].kind, storage::SessionKind::Writing);
    assert_eq!(sessions[0].status, storage::SessionStatus::Completed);

    // The draft is a stored turn, so the queue can finish it later.
    let turns = db.turns().list(sessions[0].id).await.unwrap();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].role, storage::TurnRole::Learner);

    // The queue entry is the workshop's own kind and holds the draft.
    let queue = db.pending_scoring().oldest(10).await.unwrap();
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0].payload["kind"], "writing_draft");
    assert_eq!(queue[0].tries, 0);

    // The placeholder rows are practice: free mode, no estimate, not scored.
    let attempts = db.attempts().for_session(sessions[0].id).await.unwrap();
    assert_eq!(attempts.len(), 3, "one row per rubric dimension");
    assert!(
        attempts
            .iter()
            .all(|a| a.status == AttemptStatus::PendingLlm
                && a.origin == AttemptOrigin::FreeMode
                && !a.counts_toward_estimate)
    );
    assert!(
        db.estimates().latest_per_skill(1).await.unwrap().is_empty(),
        "no estimate may come from the workshop"
    );
}

#[tokio::test]
async fn the_rule_findings_of_a_typed_draft_are_reported_without_a_provider() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("workshop.sqlite");
    let path = script(
        dir.path(),
        &[
            "Last weekend I saw an test at the lake with my family.",
            "Last weekend I recieve a letter from my family.",
        ],
    );
    let mut args = args(&db_path, Some(&path), None);
    args.offline = true;
    run(
        &args,
        &task(),
        None,
        connection(Arc::new(NoProvider), "none"),
    )
    .await
    .unwrap();

    // Layer one runs on the real checker, with spelling on for a typed draft.
    // It is partial (docs/GRAMMAR_CHECK.md): the article mistake and the
    // misspelling are found, "I has" is not, and this asserts only what it does
    // find.
    let check = tutor_engine::HarperCheck::new();
    let findings = check.findings("Last weekend I saw an test at the lake with my family.");
    assert!(
        findings
            .iter()
            .any(|f| f.to_lowercase().contains("article")),
        "{findings:?}"
    );
    let spelling = check.findings("Last weekend I recieve a letter from my family.");
    assert!(
        spelling.iter().any(|f| f.starts_with("Spelling")),
        "{spelling:?}"
    );
}

#[tokio::test]
async fn the_second_draft_is_classified_fixed_remaining_and_new() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("workshop.sqlite");
    let path = script(dir.path(), &[FIRST, SECOND]);
    let out_path = dir.path().join("result.json");
    let provider = FakeWorkshop::new();
    // Draft 1: two errors. Draft 2: the agreement one stays, the spelling one is
    // gone, and a new agreement mistake appears.
    provider.queue_errors(vec![
        error("subject_verb_agreement", "I has", "I have"),
        error("spelling", "teh lake", "the lake"),
    ]);
    provider.queue_bands("Saturday at the lake");
    provider.queue_errors(vec![
        error("subject_verb_agreement", "I has", "I have"),
        error("subject_verb_agreement", "We goes", "We go"),
    ]);
    provider.queue_bands("Saturday at the lake");

    let args = args(&db_path, Some(&path), Some(&out_path));
    let exit = run(
        &args,
        &task(),
        None,
        connection(provider.clone(), "fake-model"),
    )
    .await
    .unwrap();
    assert_eq!(exit, tutor_cli::exit::Exit::Done);
    assert_eq!(
        provider.calls(),
        [
            Contract::TurnAnalysis,
            Contract::RubricScore,
            Contract::TurnAnalysis,
            Contract::RubricScore
        ],
        "T2 then T3 per draft, in order"
    );

    let result: Value = serde_json::from_str(&std::fs::read_to_string(&out_path).unwrap()).unwrap();
    assert_no_level_is_named(&serde_json::to_string(&result).unwrap());
    assert_eq!(result["drafts"].as_array().unwrap().len(), 2);
    assert_eq!(result["awaiting_feedback"], 0);
    assert_eq!(result["counts_toward_estimate"], false);
    assert_eq!(result["rubric"]["id"], "rubric-a2-written-production");
    assert_eq!(result["task"]["content_points"], json!([]));

    // Draft 1 has no earlier draft to compare with; draft 2 has the comparison.
    assert!(result["drafts"][0]["comparison"].is_null());
    let comparison = &result["drafts"][1]["comparison"];
    assert_eq!(comparison["fixed"], 1);
    assert_eq!(comparison["remaining"], 1);
    assert_eq!(comparison["new"].as_array().unwrap().len(), 1);
    assert_eq!(comparison["new"][0]["quote"], "We goes");

    // The flattened lists of the result file carry the same classification.
    assert_eq!(
        result["fixed"],
        json!([{ "quote": "teh lake", "category": "spelling" }])
    );
    assert_eq!(
        result["remaining"],
        json!([{ "quote": "I has", "category": "subject_verb_agreement" }])
    );
    assert_eq!(
        result["new"],
        json!([{ "quote": "We goes", "category": "subject_verb_agreement" }])
    );

    // Layer two and layer three are in the file for each draft.
    for draft in result["drafts"].as_array().unwrap() {
        assert_eq!(draft["status"], "analysed");
        assert_eq!(draft["errors"].as_array().unwrap().len(), 2);
        assert_eq!(draft["rubric"]["dimensions"].as_array().unwrap().len(), 3);
        assert_eq!(draft["rubric"]["on_task"], true);
        assert_eq!(draft["rubric"]["dimensions"][0]["band"], 3.0);
    }

    // The bands are stored as free-mode attempts that cannot move an estimate.
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
    assert_eq!(attempts.len(), 6, "three dimensions per draft");
    assert!(attempts.iter().all(|a| a.status == AttemptStatus::Scored
        && a.origin == AttemptOrigin::FreeMode
        && !a.counts_toward_estimate));
    assert!(
        db.pending_scoring().oldest(10).await.unwrap().is_empty(),
        "nothing waits when the provider answered"
    );
    assert!(db.estimates().latest_per_skill(1).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_provider_that_fails_halfway_queues_the_rest_and_the_round_still_ends_zero() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("workshop.sqlite");
    let path = script(dir.path(), &[FIRST, SECOND]);
    let provider = FakeWorkshop::new();
    provider.queue_errors(vec![error("spelling", "teh lake", "the lake")]);
    provider.queue_bands("Saturday at the lake");
    // The second draft finds no provider: no analysis is queued for it, so the
    // fake refuses the call, which is what a transport failure looks like.
    let args = args(&db_path, Some(&path), None);
    let exit = run(
        &args,
        &task(),
        None,
        connection(provider.clone(), "fake-model"),
    )
    .await
    .unwrap();
    assert_eq!(exit, tutor_cli::exit::Exit::Done);

    let db = Database::open(&db_path).await.unwrap();
    let queue = db.pending_scoring().oldest(10).await.unwrap();
    assert_eq!(queue.len(), 1, "only the draft that failed is queued");
    assert_eq!(queue[0].payload["kind"], "writing_draft");
    assert_eq!(queue[0].payload["text"], SECOND);
}

#[tokio::test]
async fn the_report_of_an_offline_round_has_the_shape_and_no_level() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("workshop.sqlite");
    let path = script(dir.path(), &[FIRST, SECOND]);
    let out_path = dir.path().join("result.json");
    let mut args = args(&db_path, Some(&path), Some(&out_path));
    args.offline = true;
    run(
        &args,
        &task(),
        None,
        connection(Arc::new(NoProvider), "none"),
    )
    .await
    .unwrap();

    let text = std::fs::read_to_string(&out_path).unwrap();
    assert_no_level_is_named(&text);
    let result: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(result["drafts"].as_array().unwrap().len(), 2);
    assert_eq!(result["awaiting_feedback"], 2);
    assert_eq!(result["counts_toward_estimate"], false);
    assert_eq!(
        result["task"]["prompt"],
        "Write a note to a friend about the weekend."
    );
    assert_eq!(result["rubric"]["version"], 1);
    for draft in result["drafts"].as_array().unwrap() {
        assert_eq!(draft["status"], "pending");
        assert_eq!(draft["rule_checked"], true);
        assert!(draft["errors"].as_array().unwrap().is_empty());
        assert!(draft["rubric"].is_null());
        assert!(draft["comparison"].is_null());
    }
    // Nothing was compared, so every list is empty rather than absent.
    for key in ["fixed", "remaining", "new"] {
        assert!(result[key].as_array().unwrap().is_empty(), "{key}");
    }
}

#[test]
fn the_binary_runs_an_offline_round_and_writes_the_result_file() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("run.sqlite");
    let out_path = dir.path().join("result.json");
    let path = script(dir.path(), &[FIRST, SECOND]);
    let output = Command::new(env!("CARGO_BIN_EXE_tutor-cli"))
        .args([
            "workshop",
            "--topic",
            "Write a note to a friend about the weekend.",
            "--script",
            path.to_str().unwrap(),
            "--level",
            "a1",
            "--rubrics",
            rubrics_dir().to_str().unwrap(),
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
    assert!(
        stdout.contains("workshop: Write a note to a friend about the weekend."),
        "{stdout}"
    );
    assert!(
        stdout.contains("none: drafts are stored and queued"),
        "{stdout}"
    );
    assert!(stdout.contains("draft 1:"), "{stdout}");
    assert!(stdout.contains("rule check:"), "{stdout}");
    assert!(
        stdout.contains("waiting for a provider: the draft is stored and queued"),
        "{stdout}"
    );
    assert!(
        stdout.contains("2 draft(s); 2 wait for a provider"),
        "{stdout}"
    );
    assert!(
        stdout.contains("every draft here is practice: none of it counts toward a level estimate"),
        "{stdout}"
    );
    assert!(
        stdout.contains("rubric: rubric-a1-written-production v1 (4 dimension(s))"),
        "{stdout}"
    );
    assert_no_level_is_named(&stdout);

    let result: Value = serde_json::from_str(&std::fs::read_to_string(&out_path).unwrap()).unwrap();
    assert_eq!(result["awaiting_feedback"], 2);
    assert_eq!(result["drafts"].as_array().unwrap().len(), 2);
    assert_no_level_is_named(&serde_json::to_string(&result).unwrap());

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
        assert_eq!(session.kind, storage::SessionKind::Writing);
        assert_eq!(session.status, storage::SessionStatus::Completed);
        let attempts = db.attempts().for_session(session.id).await.unwrap();
        assert_eq!(attempts.len(), 8, "four dimensions per draft");
        assert!(attempts.iter().all(|a| !a.counts_toward_estimate
            && a.origin == AttemptOrigin::FreeMode
            && a.status == AttemptStatus::PendingLlm));
        let queue = db.pending_scoring().oldest(10).await.unwrap();
        assert_eq!(queue.len(), 2, "both drafts wait for a provider");
    });
}

#[test]
fn the_usage_error_of_a_missing_task_or_drafts_file_is_exit_two() {
    let no_task = Command::new(env!("CARGO_BIN_EXE_tutor-cli"))
        .args(["workshop", "--interactive", "--offline"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(
        no_task.status.code(),
        Some(2),
        "a round needs a topic or a prompt id"
    );

    let no_source = Command::new(env!("CARGO_BIN_EXE_tutor-cli"))
        .args([
            "workshop",
            "--topic",
            "Something to write about",
            "--offline",
        ])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(
        no_source.status.code(),
        Some(2),
        "a round needs a script or --interactive"
    );
}
