//! The `anthropic_messages` adapter (`docs/PROMPT_CONTRACTS.md` section 3.2).
//!
//! Streaming is decoded by event type. Only `text_delta` carries reply text;
//! thinking, signature and tool-input deltas are ignored. `stop_reason` arrives in
//! `message_delta` and maps to `FinishReason`, with `refusal` and `max_tokens`
//! kept distinct because both can produce output that does not match a schema.
//!
//! Structured output uses `output_config.format` (ladder level 1). The forced tool
//! call (level 2) is the fallback for endpoints that do not implement it. Note
//! that the newest Claude models reject forced `tool_choice` with HTTP 400, which
//! is harmless here because those models support level 1.

use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

use crate::adapter::{
    AdapterConfig, Completion, CompletionRequest, Format, ProtocolAdapter, endpoint_url,
};
use crate::error::{LlmError, TransportKind};
use crate::http::map_reqwest_error;
use crate::inspector::{PayloadLog, PayloadOutcome};
use crate::openai::temperature_value;
use crate::profile::Protocol;
use crate::redact::sanitize_message;
use crate::sse::SseEvent;
use crate::stream::{Decoded, EventDecoder, StreamGuards, sse_text_stream};
use crate::transport::{CallBudget, Posted, Transport, rate_limit_from_headers, read_reply};
use crate::types::{ChatMessage, FinishReason, RateLimit, Role, TextRequest, TextStream, Usage};

const ANTHROPIC_VERSION: &str = "2023-06-01";
const MAX_ADJUSTMENTS: usize = 2;

#[derive(Debug)]
pub struct AnthropicMessages {
    config: AdapterConfig,
    transport: Transport,
}

impl AnthropicMessages {
    /// `config.base_url` is used as given; `/v1/messages` is appended, so the base
    /// URL of the Anthropic API has no `/v1`.
    pub fn new(config: AdapterConfig) -> Result<Self, LlmError> {
        let url = endpoint_url(&config.base_url, "v1/messages")?;
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("anthropic-version"),
            HeaderValue::from_static(ANTHROPIC_VERSION),
        );
        if let Some(key) = &config.key {
            let mut value = HeaderValue::from_str(key.expose()).map_err(|_| {
                LlmError::InvalidRequest("the key cannot be sent as an HTTP header".to_owned())
            })?;
            value.set_sensitive(true);
            headers.insert(HeaderName::from_static("x-api-key"), value);
        }
        let transport = Transport::new(
            config.http.clone(),
            url,
            headers,
            config.key.clone(),
            config.options.retry,
        );
        Ok(Self { config, transport })
    }

    /// Records every request and response of this adapter in `log`, for the
    /// payload inspector.
    #[must_use]
    pub fn with_payload_log(mut self, log: PayloadLog) -> Self {
        self.transport = self.transport.with_log(log);
        self
    }

    /// Sends the request. When the server answers HTTP 400 and names `temperature`,
    /// the request is repeated without it, and the omission is stored only after
    /// that request succeeded.
    async fn send(
        &self,
        request: &CompletionRequest<'_>,
        stream: bool,
        budget: &CallBudget,
        cancel: &CancellationToken,
    ) -> Result<Posted, LlmError> {
        let mut send_temperature = self.config.caps.snapshot().supports_temperature;
        let before = send_temperature;
        for _ in 0..MAX_ADJUSTMENTS {
            let body = build_body(&self.config.model, request, stream, send_temperature)?;
            match self.transport.post(&body, stream, budget, cancel).await {
                Ok(response) => {
                    if send_temperature != before {
                        self.config
                            .caps
                            .update(|caps| caps.supports_temperature = false);
                    }
                    return Ok(response);
                }
                Err(LlmError::Rejected {
                    status,
                    message,
                    param,
                }) => {
                    let names_temperature =
                        message.contains("temperature") || param.as_deref() == Some("temperature");
                    if request.temperature.is_some() && send_temperature && names_temperature {
                        send_temperature = false;
                        tracing::debug!(
                            target: "llm_client::anthropic",
                            "the server rejected temperature; retrying without it"
                        );
                    } else {
                        return Err(LlmError::Rejected {
                            status,
                            message,
                            param,
                        });
                    }
                }
                Err(other) => return Err(other),
            }
        }
        Err(LlmError::Protocol(
            "the provider kept rejecting the adjusted request".to_owned(),
        ))
    }
}

fn role_name(role: Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Assistant => "assistant",
    }
}

fn build_body(
    model: &str,
    request: &CompletionRequest<'_>,
    stream: bool,
    send_temperature: bool,
) -> Result<Value, LlmError> {
    let messages: Vec<Value> = request
        .messages
        .iter()
        .map(
            |ChatMessage { role, content }| json!({ "role": role_name(*role), "content": content }),
        )
        .collect();

    let mut body = Map::new();
    body.insert("model".to_owned(), json!(model));
    body.insert("max_tokens".to_owned(), json!(request.max_tokens));
    if !request.system.is_empty() {
        body.insert("system".to_owned(), json!(request.system));
    }
    body.insert("messages".to_owned(), Value::Array(messages));
    body.insert("stream".to_owned(), json!(stream));
    if let Some(temperature) = request.temperature.filter(|_| send_temperature) {
        body.insert("temperature".to_owned(), temperature_value(temperature));
    }
    match request.format {
        Format::Plain => {}
        Format::NativeSchema(schema) => {
            body.insert(
                "output_config".to_owned(),
                json!({ "format": { "type": "json_schema", "schema": schema.schema } }),
            );
        }
        Format::ForcedTool(schema) => {
            body.insert(
                "tools".to_owned(),
                json!([{
                    "name": schema.name,
                    "description": "Return the result in this exact structure.",
                    "input_schema": schema.schema
                }]),
            );
            body.insert(
                "tool_choice".to_owned(),
                json!({ "type": "tool", "name": schema.name }),
            );
        }
        Format::JsonMode => {
            return Err(LlmError::InvalidRequest(
                "JSON mode does not exist in the Anthropic protocol".to_owned(),
            ));
        }
    }
    Ok(Value::Object(body))
}

fn map_stop_reason(reason: &str) -> FinishReason {
    match reason {
        "end_turn" | "stop_sequence" => FinishReason::Stop,
        "max_tokens" | "model_context_window_exceeded" => FinishReason::Length,
        "refusal" => FinishReason::Refusal,
        "tool_use" => FinishReason::ToolCalls,
        other => FinishReason::Other(other.to_owned()),
    }
}

fn token_count(value: Option<&Value>) -> Option<u32> {
    value
        .and_then(Value::as_u64)
        .map(|n| u32::try_from(n).unwrap_or(u32::MAX))
}

fn parse_usage(value: &Value) -> Option<Usage> {
    let usage = value.as_object()?;
    let parsed = Usage {
        input_tokens: token_count(usage.get("input_tokens")),
        output_tokens: token_count(usage.get("output_tokens")),
    };
    (parsed.input_tokens.is_some() || parsed.output_tokens.is_some()).then_some(parsed)
}

fn error_message(error: &Value) -> String {
    let kind = error.get("type").and_then(Value::as_str).unwrap_or("error");
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("unspecified");
    format!("{kind}: {message}")
}

#[derive(Debug, Default)]
struct AnthropicDecoder {
    usage: Usage,
}

impl AnthropicDecoder {
    /// `message_start` carries the input tokens and `message_delta` the running
    /// output tokens; keep the latest value of each and report the pair.
    fn absorb_usage(&mut self, value: Option<&Value>) -> Option<Decoded> {
        let parsed = value.and_then(parse_usage)?;
        self.usage.input_tokens = parsed.input_tokens.or(self.usage.input_tokens);
        self.usage.output_tokens = parsed.output_tokens.or(self.usage.output_tokens);
        Some(Decoded::Usage(self.usage))
    }
}

impl EventDecoder for AnthropicDecoder {
    fn decode(&mut self, event: &SseEvent) -> Result<Vec<Decoded>, LlmError> {
        let data: Value = serde_json::from_str(event.data.trim())
            .map_err(|_| LlmError::Protocol("a stream event was not valid JSON".to_owned()))?;
        // The `type` field of the payload is authoritative; the `event:` line repeats it.
        let kind = data
            .get("type")
            .and_then(Value::as_str)
            .or(event.event.as_deref())
            .unwrap_or("");
        let mut out = Vec::new();
        match kind {
            "message_start" => {
                out.extend(self.absorb_usage(data.get("message").and_then(|m| m.get("usage"))));
            }
            "content_block_delta" => {
                let delta = data.get("delta");
                let is_text =
                    delta.and_then(|d| d.get("type")).and_then(Value::as_str) == Some("text_delta");
                let text = delta
                    .and_then(|d| d.get("text"))
                    .and_then(Value::as_str)
                    .filter(|_| is_text);
                if let Some(text) = text {
                    out.push(Decoded::Delta(text.to_owned()));
                }
            }
            "message_delta" => {
                out.extend(self.absorb_usage(data.get("usage")));
                if let Some(reason) = data
                    .get("delta")
                    .and_then(|d| d.get("stop_reason"))
                    .and_then(Value::as_str)
                {
                    out.push(Decoded::Finish(map_stop_reason(reason)));
                }
            }
            "message_stop" => out.push(Decoded::End),
            "error" => {
                let error = data.get("error").unwrap_or(&data);
                return Err(LlmError::Stream {
                    message: error_message(error),
                });
            }
            // `ping`, block start and stop events, and event types added later.
            _ => {}
        }
        Ok(out)
    }

    fn on_close(&mut self, finish_seen: bool) -> Result<Option<FinishReason>, LlmError> {
        // A stop reason without `message_stop` is a complete reply. Without a stop
        // reason the model had not finished, so the connection broke mid-reply.
        if finish_seen {
            Ok(None)
        } else {
            Err(LlmError::Transport(TransportKind::Body))
        }
    }
}

fn parse_completion(
    body: &[u8],
    format: &Format<'_>,
    key: Option<&str>,
) -> Result<Completion, LlmError> {
    let reply: Value = serde_json::from_slice(body)
        .map_err(|_| LlmError::Protocol("the reply was not valid JSON".to_owned()))?;
    if reply.get("type").and_then(Value::as_str) == Some("error") {
        let message = reply.get("error").map(error_message).unwrap_or_default();
        return Err(LlmError::Protocol(format!(
            "the provider answered HTTP 200 with an error: {}",
            sanitize_message(&message, key)
        )));
    }
    let stop_reason = reply.get("stop_reason").and_then(Value::as_str);
    let finish = stop_reason.map_or(FinishReason::Unspecified, map_stop_reason);
    if finish == FinishReason::Refusal {
        return Err(LlmError::Refusal);
    }
    let Some(blocks) = reply.get("content").and_then(Value::as_array) else {
        // Some Anthropic-compatible gateways answer a non-streaming request
        // with the OpenAI body shape (`choices[0].message`) even though the
        // request used the Anthropic protocol. The Anthropic shape is tried
        // first and the OpenAI shape is the fallback, so such a gateway costs
        // a fallback instead of the call.
        return crate::openai::completion_from_reply(&reply, format, key);
    };
    let tool_input = match format {
        Format::ForcedTool(_) => blocks
            .iter()
            .find(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
            .and_then(|b| b.get("input"))
            .and_then(|input| serde_json::to_string(input).ok()),
        _ => None,
    };
    let text = tool_input.unwrap_or_else(|| {
        blocks
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("")
    });
    Ok(Completion {
        text,
        finish,
        usage: reply.get("usage").and_then(parse_usage),
        rate_limit: RateLimit::default(),
    })
}

#[async_trait]
impl ProtocolAdapter for AnthropicMessages {
    fn protocol(&self) -> Protocol {
        Protocol::AnthropicMessages
    }

    async fn stream_text(
        &self,
        request: &TextRequest,
        cancel: &CancellationToken,
    ) -> Result<TextStream, LlmError> {
        let budget = CallBudget::start(&self.config.options.tutor);
        let completion_request = CompletionRequest {
            system: &request.system,
            messages: &request.messages,
            max_tokens: request.max_tokens,
            temperature: request.temperature,
            format: Format::Plain,
        };
        let Posted { response, capture } = self
            .send(&completion_request, true, &budget, cancel)
            .await?;
        let bytes = response
            .bytes_stream()
            .map(|chunk| chunk.map_err(|e| map_reqwest_error(&e)))
            .boxed();
        Ok(sse_text_stream(
            bytes,
            AnthropicDecoder::default(),
            StreamGuards {
                cancel: cancel.clone(),
                budget,
                key: self.config.key.clone(),
                capture,
            },
        ))
    }

    async fn complete(
        &self,
        request: &CompletionRequest<'_>,
        cancel: &CancellationToken,
    ) -> Result<Completion, LlmError> {
        let budget = CallBudget::start(&self.config.options.background);
        let Posted { response, capture } = self.send(request, false, &budget, cancel).await?;
        let rate_limit = rate_limit_from_headers(response.headers());
        let body = match read_reply(response, &budget, cancel).await {
            Ok(body) => body,
            Err(error) => {
                if let Some(capture) = capture {
                    capture.finish(PayloadOutcome::Failed);
                }
                return Err(error);
            }
        };
        if let Some(capture) = capture {
            capture.finish_with(PayloadOutcome::Ok, &body);
        }
        let mut completion = parse_completion(
            &body,
            &request.format,
            self.config.key.as_ref().map(crate::key::ApiKey::expose),
        )?;
        completion.rate_limit = rate_limit;
        Ok(completion)
    }
}
