#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
#![allow(dead_code)]

//! A language model for the sessions that need more than a chat and an analysis:
//! it also scores a rubric and writes a reading text, and it remembers every
//! request it was asked, so a test can look for what must not be in one.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use llm_client::{
    Capabilities, Contract, LadderLevel, LlmClient, LlmError, StructuredOutput, StructuredRequest,
    TextRequest, TextStream,
};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use app_core::voice::testing::ScriptedLlm;

pub struct ContractLlm {
    pub inner: Arc<ScriptedLlm>,
    /// What a reading request is answered with, once. Without one, it fails like a
    /// provider that is down.
    reading: Mutex<Vec<Value>>,
    /// Every request, as the text a provider would have received.
    seen: Mutex<Vec<String>>,
}

impl ContractLlm {
    pub fn new(inner: Arc<ScriptedLlm>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            reading: Mutex::new(Vec::new()),
            seen: Mutex::new(Vec::new()),
        })
    }

    pub fn queue_reading(&self, text: Value) {
        self.reading.lock().unwrap().push(text);
    }

    /// Every request so far: system prompt and messages joined.
    pub fn seen(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }
}

fn joined(system: &str, messages: &[llm_client::ChatMessage]) -> String {
    let mut text = system.to_owned();
    for message in messages {
        text.push('\n');
        text.push_str(&message.content);
    }
    text
}

fn rubric_reply(request: &StructuredRequest) -> Value {
    let body: Value = serde_json::from_str(&request.messages[0].content).expect("a json message");
    let response = body["response"].as_str().unwrap_or_default();
    let quote: String = response
        .split_whitespace()
        .take(3)
        .collect::<Vec<_>>()
        .join(" ");
    let dimensions: Vec<Value> = body["rubric"]
        .as_array()
        .expect("a rubric")
        .iter()
        .map(|d| {
            json!({
                "dimension": d["dimension"],
                "band": "3",
                "evidence_quotes": [quote],
                "reason": "A short reason."
            })
        })
        .collect();
    let points: Vec<Value> = body["content_points"]
        .as_array()
        .map(|points| {
            points
                .iter()
                .map(|p| json!({ "point": p, "covered": true, "quote": quote }))
                .collect()
        })
        .unwrap_or_default();
    json!({
        "dimension_scores": dimensions,
        "content_points": points,
        "on_task": true,
        "feedback_en": "You did the task. Add one more detail next time.",
        "feedback_l1": "Kamu menyelesaikan tugasnya. Tambahkan satu detail lagi."
    })
}

#[async_trait]
impl LlmClient for ContractLlm {
    async fn stream_text(
        &self,
        request: TextRequest,
        cancel: CancellationToken,
    ) -> Result<TextStream, LlmError> {
        self.seen
            .lock()
            .unwrap()
            .push(joined(&request.system, &request.messages));
        self.inner.stream_text(request, cancel).await
    }

    async fn structured(
        &self,
        request: StructuredRequest,
        cancel: CancellationToken,
    ) -> Result<StructuredOutput, LlmError> {
        self.seen
            .lock()
            .unwrap()
            .push(joined(&request.system, &request.messages));
        let value = match request.contract {
            Contract::RubricScore => rubric_reply(&request),
            Contract::ReadingPassage => {
                let mut queue = self.reading.lock().unwrap();
                if queue.is_empty() {
                    return Err(LlmError::Transport(llm_client::TransportKind::Connect));
                }
                queue.remove(0)
            }
            _ => return self.inner.structured(request, cancel).await,
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
        self.inner.capabilities()
    }
}

/// A valid A1 reading text: 84 plain words, five questions of which the level
/// keeps four.
pub fn reading_text() -> Value {
    let question = |stem: &str, answer: i64| {
        json!({ "stem": stem, "options": ["fruit", "shoes", "books"], "answer_index": answer,
                "explanation_en": "See the first line.", "explanation_l1": "Lihat baris pertama." })
    };
    json!({
        "title": "The market",
        "passage": "The market sells fresh fruit every morning. ".repeat(12),
        "glossary": [{ "word": "fresh fruit", "gloss_l1": "buah segar", "example": "I like fresh fruit." }],
        "questions": [
            question("What does the market sell?", 0),
            question("When does it sell?", 0),
            question("Where is it?", 1),
            question("Who goes there?", 0),
            question("A fifth question the level does not need?", 0)
        ]
    })
}
