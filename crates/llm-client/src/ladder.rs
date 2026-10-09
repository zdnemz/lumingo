//! The structured-output ladder (`docs/PROMPT_CONTRACTS.md` section 5) on top of a
//! protocol adapter.
//!
//! A structured call starts at the level the probe cached. The reply is parsed,
//! validated against the contract, and repaired once if it fails. A request the
//! provider rejects outright (the level is not supported) moves the call down one
//! level and caches the level that worked. A reply that is still invalid after the
//! repair is `invalid_output`; the ladder does not walk further down for it,
//! because that would triple the latency of a bad answer.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::adapter::{Completion, CompletionRequest, Format, ProtocolAdapter, SchemaRef};
use crate::caps::CapsHandle;
use crate::client::LlmClient;
use crate::error::{InvalidOutput, LlmError};
use crate::types::{
    Capabilities, ChatMessage, FinishReason, LadderLevel, StructuredOutput, StructuredRequest,
    TextRequest, TextStream, Usage,
};
use crate::validate::{CompiledSchema, Invalid, Violation, contract_schema};

/// Largest `max_tokens` a retry after `finish_reason: length` asks for.
const MAX_TOKENS_CEILING: u32 = 16_384;
/// Calls remembered for the validity rate. `docs/PROMPT_CONTRACTS.md` section 5.
pub const VALIDITY_WINDOW: usize = 50;
/// A profile below this validity rate over the window is re-probed.
pub const REPROBE_BELOW: f64 = 0.9;
/// Longest invalid reply echoed back to the model in a repair call.
const MAX_ECHOED_REPLY_CHARS: usize = 8_000;
const MAX_LISTED_VIOLATIONS: usize = 10;
const MAX_VIOLATION_CHARS: usize = 200;

/// One adapter plus the shared capabilities, behind the `LlmClient` trait.
pub struct ProviderClient {
    pub(crate) adapter: Arc<dyn ProtocolAdapter>,
    pub(crate) caps: CapsHandle,
    first_pass_validity: Mutex<VecDeque<bool>>,
}

impl std::fmt::Debug for ProviderClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderClient")
            .field("protocol", &self.adapter.protocol())
            .finish_non_exhaustive()
    }
}

/// The parts of a request that stay the same across ladder levels and the repair call.
pub(crate) struct CallSpec<'a> {
    pub system: &'a str,
    pub messages: &'a [ChatMessage],
    pub max_tokens: u32,
    pub temperature: Option<f32>,
}

/// What one successful call (with its repair, if any) produced.
struct Validated {
    value: Value,
    repaired: bool,
}

#[derive(Default)]
struct Tally {
    calls: u8,
    usage: Option<Usage>,
}

impl Tally {
    /// Counts a request when it is sent, so a request the provider rejects is
    /// counted too: the ladder's descent costs requests.
    fn sent(&mut self) {
        self.calls = self.calls.saturating_add(1);
    }

    fn add_tokens(&mut self, completion: &Completion) {
        if let Some(usage) = completion.usage {
            let current = self.usage.get_or_insert_with(Usage::default);
            current.input_tokens = sum(current.input_tokens, usage.input_tokens);
            current.output_tokens = sum(current.output_tokens, usage.output_tokens);
        }
    }
}

fn sum(a: Option<u32>, b: Option<u32>) -> Option<u32> {
    match (a, b) {
        (None, None) => None,
        (a, b) => Some(a.unwrap_or(0).saturating_add(b.unwrap_or(0))),
    }
}

impl ProviderClient {
    /// Wraps an adapter. `caps` must be the handle the adapter was built with, so
    /// the quirks it learns and the probe results end up in one place.
    pub fn from_adapter(adapter: Arc<dyn ProtocolAdapter>, caps: CapsHandle) -> Self {
        Self {
            adapter,
            caps,
            first_pass_validity: Mutex::new(VecDeque::with_capacity(VALIDITY_WINDOW)),
        }
    }

    /// Forces the structured-output ladder level. A normal run never calls this:
    /// the level comes from the probe. The live checks use it to compare what a
    /// provider does at each level (`TUTOR_LLM_FORCE_LEVEL=1..4`).
    pub fn set_structured_level(&self, level: LadderLevel) {
        self.caps.update(|caps| caps.structured_level = Some(level));
    }

    /// Share of the last structured replies that were valid on the first try, once
    /// `VALIDITY_WINDOW` replies have been seen.
    pub fn validity_rate(&self) -> Option<f64> {
        let window = self
            .first_pass_validity
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if window.len() < VALIDITY_WINDOW {
            return None;
        }
        let valid = window.iter().filter(|ok| **ok).count();
        Some(valid as f64 / window.len() as f64)
    }

    /// True when the validity rate over the last 50 structured calls fell below 90 percent.
    /// The first-try rate is used, not the rate after repair, because a repair hides a
    /// provider that has started to drift.
    pub fn needs_reprobe(&self) -> bool {
        self.validity_rate()
            .is_some_and(|rate| rate < REPROBE_BELOW)
    }

    pub(crate) fn clear_validity(&self) {
        self.first_pass_validity
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    fn record_first_pass(&self, valid: bool) {
        let mut window = self
            .first_pass_validity
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if window.len() == VALIDITY_WINDOW {
            window.pop_front();
        }
        window.push_back(valid);
    }

    /// One provider request at `level`. Levels 3 and 4 carry the schema text in the
    /// system prompt; levels 1 and 2 send the schema in the request envelope.
    pub(crate) async fn call_level(
        &self,
        level: LadderLevel,
        spec: &CallSpec<'_>,
        messages: &[ChatMessage],
        max_tokens: u32,
        schema: &CompiledSchema,
        cancel: &CancellationToken,
    ) -> Result<Completion, LlmError> {
        if cancel.is_cancelled() {
            return Err(LlmError::Cancelled);
        }
        let schema_ref = SchemaRef {
            name: schema.name,
            schema: &schema.wire,
        };
        let system_with_schema;
        let (system, format) = match level {
            LadderLevel::NativeSchema => (spec.system, Format::NativeSchema(schema_ref)),
            LadderLevel::ForcedTool => (spec.system, Format::ForcedTool(schema_ref)),
            LadderLevel::JsonMode => {
                system_with_schema = with_schema_text(spec.system, schema);
                (system_with_schema.as_str(), Format::JsonMode)
            }
            LadderLevel::PromptOnly => {
                system_with_schema = with_schema_text(spec.system, schema);
                (system_with_schema.as_str(), Format::Plain)
            }
        };
        let request = CompletionRequest {
            system,
            messages,
            max_tokens,
            temperature: spec.temperature,
            format,
        };
        self.adapter.complete(&request, cancel).await
    }

    /// One call at `level` with the length retry and the single repair.
    async fn call_validated(
        &self,
        level: LadderLevel,
        spec: &CallSpec<'_>,
        schema: &CompiledSchema,
        cancel: &CancellationToken,
        tally: &mut Tally,
    ) -> Result<Validated, LlmError> {
        let mut max_tokens = spec.max_tokens;
        tally.sent();
        let mut completion = self
            .call_level(level, spec, spec.messages, max_tokens, schema, cancel)
            .await?;
        tally.add_tokens(&completion);

        // `finish_reason: length` means the JSON is cut off. One retry with a larger limit.
        if completion.finish == FinishReason::Length && max_tokens < MAX_TOKENS_CEILING {
            max_tokens = max_tokens.saturating_mul(2).min(MAX_TOKENS_CEILING);
            tally.sent();
            completion = self
                .call_level(level, spec, spec.messages, max_tokens, schema, cancel)
                .await?;
            tally.add_tokens(&completion);
        }

        let invalid = match schema.check_text(&completion.text) {
            Ok(value) => {
                self.record_first_pass(true);
                return Ok(Validated {
                    value,
                    repaired: false,
                });
            }
            Err(invalid) => invalid,
        };
        self.record_first_pass(false);

        let mut repair_messages = spec.messages.to_vec();
        repair_messages.push(ChatMessage::assistant(truncate_chars(
            &completion.text,
            MAX_ECHOED_REPLY_CHARS,
        )));
        repair_messages.push(ChatMessage::user(repair_request(&invalid)));
        tally.sent();
        let repaired = self
            .call_level(level, spec, &repair_messages, max_tokens, schema, cancel)
            .await?;
        tally.add_tokens(&repaired);
        match schema.check_text(&repaired.text) {
            Ok(value) => Ok(Validated {
                value,
                repaired: true,
            }),
            Err(still_invalid) => Err(LlmError::InvalidOutput(InvalidOutput {
                reason: still_invalid.reason,
                paths: still_invalid
                    .violations
                    .iter()
                    .map(|v| v.path.clone())
                    .collect(),
                ladder_level: level,
                repaired: true,
            })),
        }
    }

    async fn structured_call(
        &self,
        request: &StructuredRequest,
        cancel: &CancellationToken,
    ) -> Result<StructuredOutput, LlmError> {
        let schema = contract_schema(request.contract);
        let spec = CallSpec {
            system: &request.system,
            messages: &request.messages,
            max_tokens: request.max_tokens,
            temperature: request.temperature,
        };
        let cached = self.caps.snapshot().structured_level;
        let mut level = cached.unwrap_or(LadderLevel::NativeSchema);
        let mut tally = Tally::default();
        loop {
            match self
                .call_validated(level, &spec, schema, cancel, &mut tally)
                .await
            {
                Ok(validated) => {
                    if cached != Some(level) {
                        self.caps.update(|caps| caps.structured_level = Some(level));
                    }
                    return Ok(StructuredOutput {
                        value: validated.value,
                        ladder_level: level,
                        repaired: validated.repaired,
                        calls: tally.calls,
                        usage: tally.usage,
                    });
                }
                // The provider refused the request itself, so this level is not
                // supported there. Try the next one.
                Err(error @ LlmError::Rejected { .. }) => match level.next(self.adapter.protocol())
                {
                    Some(next) => {
                        tracing::debug!(
                            target: "llm_client::ladder",
                            from = level.as_u8(),
                            to = next.as_u8(),
                            "structured output level rejected; trying the next level"
                        );
                        level = next;
                    }
                    None => return Err(error),
                },
                Err(other) => return Err(other),
            }
        }
    }
}

fn truncate_chars(text: &str, limit: usize) -> String {
    match text.char_indices().nth(limit) {
        Some((end, _)) => text[..end].to_owned(),
        None => text.to_owned(),
    }
}

/// System prompt addition for levels 3 and 4, where the provider is not given the
/// schema in the request envelope.
pub(crate) fn with_schema_text(system: &str, schema: &CompiledSchema) -> String {
    format!(
        "{system}\n\nReturn only one JSON object that matches this JSON Schema. Do not add markdown, code fences or any other text.\nSCHEMA\n{}",
        schema.wire
    )
}

fn repair_request(invalid: &Invalid) -> String {
    use crate::error::InvalidReason;

    let mut text = match invalid.reason {
        InvalidReason::NotJson => "Your last reply did not contain a JSON value.".to_owned(),
        InvalidReason::SchemaMismatch => {
            let mut text = String::from("Your last reply did not match the schema. Problems:");
            for Violation { path, message } in invalid.violations.iter().take(MAX_LISTED_VIOLATIONS)
            {
                let place = if path.is_empty() {
                    "(root)"
                } else {
                    path.as_str()
                };
                text.push_str(&format!(
                    "\n- {place}: {}",
                    truncate_chars(message, MAX_VIOLATION_CHARS)
                ));
            }
            text
        }
    };
    text.push_str("\nReply again with only the corrected JSON, matching the schema.");
    text
}

#[async_trait]
impl LlmClient for ProviderClient {
    async fn stream_text(
        &self,
        request: TextRequest,
        cancel: CancellationToken,
    ) -> Result<TextStream, LlmError> {
        self.adapter.stream_text(&request, &cancel).await
    }

    async fn structured(
        &self,
        request: StructuredRequest,
        cancel: CancellationToken,
    ) -> Result<StructuredOutput, LlmError> {
        self.structured_call(&request, &cancel).await
    }

    fn capabilities(&self) -> Capabilities {
        self.caps.snapshot()
    }
}

#[cfg(test)]
mod tests;
