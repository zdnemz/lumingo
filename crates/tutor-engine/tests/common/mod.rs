#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
// Each test binary uses a different subset of these helpers.
#![allow(dead_code)]

//! Test-only doubles and helpers. Nothing here is compiled into a release build:
//! it lives under `tests/`.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use llm_client::{
    Capabilities, Contract, FinishReason, LadderLevel, LlmClient, LlmError, StreamEvent,
    StreamSummary, StructuredOutput, StructuredRequest, TextRequest, TextStream,
};
use serde_json::Value;
use storage::{
    Database, InputMode, L1HelpMode, NewProfile, NewSession, NewTurn, Profile, Session,
    SessionKind, Timestamp, Turn, TurnRole, UiLanguage,
};
use tokio_util::sync::CancellationToken;
use tutor_engine::Clock;

/// What the fake does for one streamed call.
pub enum TextReply {
    /// These deltas, then a normal finish.
    Deltas(Vec<&'static str>),
    /// The call itself fails.
    Fail(LlmError),
    /// A stream that ends with a refusal and no text.
    Refusal,
    /// These deltas, then the stream breaks with this error.
    Broken(Vec<&'static str>, LlmError),
    /// A stream that ends normally with no text at all.
    Empty,
}

/// A scripted `LlmClient`. Every reply is queued in advance; a call with
/// nothing queued fails loudly so a test cannot pass by accident.
pub struct FakeLlm {
    structured: Mutex<VecDeque<Result<Value, LlmError>>>,
    texts: Mutex<VecDeque<TextReply>>,
    pub structured_seen: Mutex<Vec<StructuredRequest>>,
    pub text_seen: Mutex<Vec<TextRequest>>,
}

impl FakeLlm {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            structured: Mutex::new(VecDeque::new()),
            texts: Mutex::new(VecDeque::new()),
            structured_seen: Mutex::new(Vec::new()),
            text_seen: Mutex::new(Vec::new()),
        })
    }

    pub fn queue_structured(&self, reply: Result<Value, LlmError>) {
        if let Ok(value) = &reply {
            // The real client validates before it returns; the fake checks the
            // test data against the same schema so a fixture cannot drift.
            let schema = Contract::ALL
                .iter()
                .find(|c| value_looks_like(c, value))
                .map(|c| c.schema());
            if let Some(schema) = schema {
                let validator = jsonschema::validator_for(schema).expect("schema compiles");
                assert!(
                    validator.is_valid(value),
                    "fixture breaks its contract: {value}"
                );
            }
        }
        self.structured.lock().unwrap().push_back(reply);
    }

    pub fn queue_text(&self, reply: TextReply) {
        self.texts.lock().unwrap().push_back(reply);
    }

    /// A copy of the structured requests seen so far, so no lock is held in a test.
    pub fn structured_requests(&self) -> Vec<StructuredRequest> {
        self.structured_seen.lock().unwrap().clone()
    }

    pub fn text_requests(&self) -> Vec<TextRequest> {
        self.text_seen.lock().unwrap().clone()
    }

    pub fn structured_calls(&self) -> usize {
        self.structured_seen.lock().unwrap().len()
    }

    pub fn text_calls(&self) -> usize {
        self.text_seen.lock().unwrap().len()
    }
}

fn value_looks_like(contract: &Contract, value: &Value) -> bool {
    let key = match contract {
        Contract::TurnAnalysis => "turns",
        Contract::PracticeItems => "items",
        Contract::ReadingPassage => "passage",
        Contract::RubricScore => "dimension_scores",
    };
    value.get(key).is_some()
}

#[async_trait]
impl LlmClient for FakeLlm {
    async fn stream_text(
        &self,
        request: TextRequest,
        _cancel: CancellationToken,
    ) -> Result<TextStream, LlmError> {
        self.text_seen.lock().unwrap().push(request);
        let reply = self
            .texts
            .lock()
            .unwrap()
            .pop_front()
            .expect("no streamed reply queued");
        let summary = |finish| {
            Ok(StreamEvent::Finished(StreamSummary {
                finish,
                usage: None,
                time_to_first_token: Some(Duration::from_millis(5)),
            }))
        };
        match reply {
            TextReply::Fail(error) => Err(error),
            TextReply::Empty => Ok(TextStream::new(futures_util::stream::iter(vec![summary(
                FinishReason::Stop,
            )]))),
            TextReply::Broken(parts, error) => {
                let mut events: Vec<Result<StreamEvent, LlmError>> = parts
                    .into_iter()
                    .map(|p| Ok(StreamEvent::Delta(p.to_owned())))
                    .collect();
                events.push(Err(error));
                Ok(TextStream::new(futures_util::stream::iter(events)))
            }
            TextReply::Refusal => Ok(TextStream::new(futures_util::stream::iter(vec![summary(
                FinishReason::Refusal,
            )]))),
            TextReply::Deltas(parts) => {
                let mut events: Vec<Result<StreamEvent, LlmError>> = parts
                    .into_iter()
                    .map(|p| Ok(StreamEvent::Delta(p.to_owned())))
                    .collect();
                events.push(summary(FinishReason::Stop));
                Ok(TextStream::new(futures_util::stream::iter(events)))
            }
        }
    }

    async fn structured(
        &self,
        request: StructuredRequest,
        _cancel: CancellationToken,
    ) -> Result<StructuredOutput, LlmError> {
        self.structured_seen.lock().unwrap().push(request);
        // An unqueued call fails like a provider would. Background tasks call this
        // without the test waiting on them, so a panic here would only be noise;
        // tests that depend on a call assert the call count.
        let reply = self
            .structured
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Err(LlmError::Protocol("no structured reply queued".into())));
        reply.map(|value| StructuredOutput {
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

/// A fresh database in its own temporary directory. Keep the `TempDir` alive.
pub async fn temp_db() -> (tempfile::TempDir, Database) {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = Database::open(dir.path().join("lumingo.sqlite"))
        .await
        .expect("open database");
    (dir, db)
}

/// A clock that moves one second per reading from a fixed start.
pub fn test_clock() -> Clock {
    let ticks = Arc::new(Mutex::new(0i64));
    Arc::new(move || {
        let mut n = ticks.lock().unwrap();
        *n += 1;
        Timestamp::from_unix_seconds(1_790_000_000 + *n).expect("timestamp")
    })
}

pub fn ts(n: i64) -> Timestamp {
    Timestamp::from_unix_seconds(1_790_000_000 + n).expect("timestamp")
}

pub async fn make_profile(db: &Database) -> Profile {
    db.profiles()
        .create(&NewProfile {
            display_name: "Test learner".to_owned(),
            ui_language: UiLanguage::Id,
            l1: "id".to_owned(),
            l1_help_mode: L1HelpMode::Auto,
            created_at: ts(0),
        })
        .await
        .expect("create profile")
}

pub async fn make_session(db: &Database, profile_id: i64, kind: SessionKind) -> Session {
    db.sessions()
        .create(&NewSession {
            profile_id,
            kind,
            unit_id: None,
            activity_id: None,
            mode: None,
            provider_profile_id: None,
            app_version: "0.0.0-test".to_owned(),
            started_at: ts(10),
        })
        .await
        .expect("create session")
}

pub async fn make_turn(db: &Database, session_id: i64, role: TurnRole, text: &str) -> Turn {
    db.turns()
        .append(&NewTurn {
            session_id,
            role,
            input_mode: InputMode::Text,
            text: text.to_owned(),
            stt_text: None,
            edited_by_learner: false,
            speech_ms: None,
            pause_ms: None,
            word_count: None,
            created_at: ts(20),
        })
        .await
        .expect("append turn")
}

/// The example unit of the curriculum, A1 unit 1.
pub fn example_unit() -> curriculum::Unit {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../curriculum/examples/a1-u01.example.json");
    curriculum::load_unit_file(&path)
        .expect("the example unit loads")
        .unit
}

/// Waits (bounded) until `check` is true. Background analysis finishes on its
/// own schedule, and a test must not sleep for a fixed time.
pub async fn wait_for(mut check: impl FnMut() -> bool) {
    for _ in 0..200 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the condition did not become true in time");
}
