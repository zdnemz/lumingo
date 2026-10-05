//! The `openai_chat` adapter (`docs/PROMPT_CONTRACTS.md` section 3.1).
//!
//! Compatibility quirks are handled here and remembered in the shared
//! capabilities, so each one costs at most one failed request per profile:
//! the token limit parameter name, a rejected `temperature`, a rejected
//! `stream_options`, keep-alive comments, empty or null deltas, a missing
//! `[DONE]`, and reasoning text in fields other than `content`.

use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::Response;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

use crate::adapter::{
    AdapterConfig, Completion, CompletionRequest, Format, ProtocolAdapter, endpoint_url,
};
use crate::error::LlmError;
use crate::http::map_reqwest_error;
use crate::profile::Protocol;
use crate::redact::sanitize_message;
use crate::sse::SseEvent;
use crate::stream::{Decoded, EventDecoder, StreamGuards, sse_text_stream};
use crate::transport::{CallBudget, Transport, rate_limit_from_headers, read_reply};
use crate::types::{
    ChatMessage, FinishReason, RateLimit, Role, TextRequest, TextStream, TokenLimitParam, Usage,
};

/// How many times one call may adjust itself to a server quirk before giving up.
const MAX_ADJUSTMENTS: usize = 4;

#[derive(Debug)]
pub struct OpenAiChat {
    config: AdapterConfig,
    transport: Transport,
}

#[derive(Debug, Clone, Copy)]
struct Quirks {
    token_param: TokenLimitParam,
    temperature: bool,
    stream_usage: bool,
}

impl OpenAiChat {
    /// `config.base_url` is used as given; `/chat/completions` is appended.
    pub fn new(config: AdapterConfig) -> Result<Self, LlmError> {
        let url = endpoint_url(&config.base_url, "chat/completions")?;
        let mut headers = HeaderMap::new();
        if let Some(key) = &config.key {
            let mut value =
                HeaderValue::from_str(&format!("Bearer {}", key.expose())).map_err(|_| {
                    LlmError::InvalidRequest("the key cannot be sent as an HTTP header".to_owned())
                })?;
            value.set_sensitive(true);
            headers.insert(AUTHORIZATION, value);
        }
        let transport = Transport::new(config.http.clone(), url, headers, config.key.clone());
        Ok(Self { config, transport })
    }

    fn quirks(&self) -> Quirks {
        let caps = self.config.caps.snapshot();
        Quirks {
            token_param: caps.token_limit_param,
            temperature: caps.supports_temperature,
            stream_usage: caps.supports_usage_in_stream,
        }
    }

    /// Sends the request, adjusting to a quirk when the server answers HTTP 400 and
    /// names the parameter. The adjustment is stored only after a request with it
    /// succeeded, so a 400 about something else cannot corrupt the cached quirks.
    async fn send(
        &self,
        request: &CompletionRequest<'_>,
        stream: bool,
        budget: &CallBudget,
        cancel: &CancellationToken,
    ) -> Result<Response, LlmError> {
        let before = self.quirks();
        let mut quirks = before;
        let mut flipped_token_param = false;
        for _ in 0..MAX_ADJUSTMENTS {
            let body = build_body(&self.config.model, request, stream, quirks);
            match self.transport.post(&body, stream, budget, cancel).await {
                Ok(response) => {
                    self.remember(before, quirks);
                    return Ok(response);
                }
                Err(LlmError::Rejected {
                    status,
                    message,
                    param,
                }) => {
                    let named =
                        |name: &str| message.contains(name) || param.as_deref() == Some(name);
                    if request.temperature.is_some() && quirks.temperature && named("temperature") {
                        quirks.temperature = false;
                    } else if stream
                        && quirks.stream_usage
                        && (named("stream_options") || named("include_usage"))
                    {
                        quirks.stream_usage = false;
                    } else if !flipped_token_param && named(quirks.token_param.wire_name()) {
                        quirks.token_param = quirks.token_param.other();
                        flipped_token_param = true;
                    } else {
                        return Err(LlmError::Rejected {
                            status,
                            message,
                            param,
                        });
                    }
                    tracing::debug!(
                        target: "llm_client::openai",
                        "the server rejected a request parameter; retrying with an adjusted request"
                    );
                }
                Err(other) => return Err(other),
            }
        }
        Err(LlmError::Protocol(
            "the provider kept rejecting the adjusted request".to_owned(),
        ))
    }

    fn remember(&self, before: Quirks, after: Quirks) {
        if before.token_param == after.token_param
            && before.temperature == after.temperature
            && before.stream_usage == after.stream_usage
        {
            return;
        }
        self.config.caps.update(|caps| {
            caps.token_limit_param = after.token_param;
            caps.supports_temperature = after.temperature;
            caps.supports_usage_in_stream = after.stream_usage;
        });
    }
}

fn role_name(role: Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Assistant => "assistant",
    }
}

/// `serde_json` would widen an `f32` such as 0.7 to 0.699999988079071. Going through
/// the decimal text keeps the number the caller wrote.
pub(crate) fn temperature_value(temperature: f32) -> Value {
    temperature
        .to_string()
        .parse::<f64>()
        .ok()
        .and_then(serde_json::Number::from_f64)
        .map_or(Value::Null, Value::Number)
}

fn build_body(model: &str, request: &CompletionRequest<'_>, stream: bool, quirks: Quirks) -> Value {
    let mut messages: Vec<Value> = Vec::with_capacity(request.messages.len() + 1);
    if !request.system.is_empty() {
        messages.push(json!({ "role": "system", "content": request.system }));
    }
    messages.extend(request.messages.iter().map(
        |ChatMessage { role, content }| json!({ "role": role_name(*role), "content": content }),
    ));

    let mut body = Map::new();
    body.insert("model".to_owned(), json!(model));
    body.insert("messages".to_owned(), Value::Array(messages));
    body.insert("stream".to_owned(), json!(stream));
    body.insert(
        quirks.token_param.wire_name().to_owned(),
        json!(request.max_tokens),
    );
    if let Some(temperature) = request.temperature.filter(|_| quirks.temperature) {
        body.insert("temperature".to_owned(), temperature_value(temperature));
    }
    if stream && quirks.stream_usage {
        body.insert(
            "stream_options".to_owned(),
            json!({ "include_usage": true }),
        );
    }
    match request.format {
        Format::Plain => {}
        Format::JsonMode => {
            body.insert(
                "response_format".to_owned(),
                json!({ "type": "json_object" }),
            );
        }
        Format::NativeSchema(schema) => {
            body.insert(
                "response_format".to_owned(),
                json!({
                    "type": "json_schema",
                    "json_schema": { "name": schema.name, "strict": true, "schema": schema.schema }
                }),
            );
        }
        Format::ForcedTool(schema) => {
            body.insert(
                "tools".to_owned(),
                json!([{
                    "type": "function",
                    "function": { "name": schema.name, "parameters": schema.schema }
                }]),
            );
            body.insert(
                "tool_choice".to_owned(),
                json!({ "type": "function", "function": { "name": schema.name } }),
            );
        }
    }
    Value::Object(body)
}

fn map_finish(reason: &str) -> FinishReason {
    match reason {
        "stop" => FinishReason::Stop,
        "length" => FinishReason::Length,
        "content_filter" => FinishReason::Refusal,
        "tool_calls" | "function_call" => FinishReason::ToolCalls,
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
        input_tokens: token_count(usage.get("prompt_tokens")),
        output_tokens: token_count(usage.get("completion_tokens")),
    };
    (parsed.input_tokens.is_some() || parsed.output_tokens.is_some()).then_some(parsed)
}

/// `content` is a string on almost every server and an array of typed parts on a few.
fn content_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

fn error_object_message(error: &Value) -> String {
    match error {
        Value::String(text) => text.clone(),
        other => other
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unspecified error")
            .to_owned(),
    }
}

#[derive(Debug, Default)]
struct OpenAiDecoder {
    refusal: bool,
}

impl EventDecoder for OpenAiDecoder {
    fn decode(&mut self, event: &SseEvent) -> Result<Vec<Decoded>, LlmError> {
        let data = event.data.trim();
        if data == "[DONE]" {
            let mut out = Vec::new();
            if self.refusal {
                out.push(Decoded::Finish(FinishReason::Refusal));
            }
            out.push(Decoded::End);
            return Ok(out);
        }
        let chunk: Value = serde_json::from_str(data)
            .map_err(|_| LlmError::Protocol("a stream chunk was not valid JSON".to_owned()))?;
        if let Some(error) = chunk.get("error").filter(|e| !e.is_null()) {
            return Err(LlmError::Stream {
                message: error_object_message(error),
            });
        }
        let mut out = Vec::new();
        if let Some(usage) = chunk.get("usage").and_then(parse_usage) {
            out.push(Decoded::Usage(usage));
        }
        // `choices` is empty in usage-only chunks and in some providers' first chunk.
        let Some(choice) = chunk.get("choices").and_then(|c| c.get(0)) else {
            return Ok(out);
        };
        let delta = choice.get("delta");
        if let Some(text) = delta.and_then(|d| d.get("content")).and_then(Value::as_str) {
            out.push(Decoded::Delta(text.to_owned()));
        }
        if delta
            .and_then(|d| d.get("refusal"))
            .and_then(Value::as_str)
            .is_some_and(|text| !text.is_empty())
        {
            self.refusal = true;
        }
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            let finish = if self.refusal {
                FinishReason::Refusal
            } else {
                map_finish(reason)
            };
            out.push(Decoded::Finish(finish));
        }
        Ok(out)
    }

    fn on_close(&mut self, finish_seen: bool) -> Result<Option<FinishReason>, LlmError> {
        // Closing without `[DONE]` is allowed by the contract; a refusal that never got
        // a finish chunk is still a refusal.
        Ok((self.refusal && !finish_seen).then_some(FinishReason::Refusal))
    }
}

fn parse_completion(
    body: &[u8],
    format: &Format<'_>,
    key: Option<&str>,
) -> Result<Completion, LlmError> {
    let reply: Value = serde_json::from_slice(body)
        .map_err(|_| LlmError::Protocol("the reply was not valid JSON".to_owned()))?;
    let Some(choice) = reply.get("choices").and_then(|c| c.get(0)) else {
        return Err(match reply.get("error").filter(|e| !e.is_null()) {
            Some(error) => LlmError::Protocol(format!(
                "the provider answered HTTP 200 with an error: {}",
                sanitize_message(&error_object_message(error), key)
            )),
            None => LlmError::Protocol("the reply has no choices".to_owned()),
        });
    };
    let message = choice.get("message").unwrap_or(&Value::Null);
    if message
        .get("refusal")
        .and_then(Value::as_str)
        .is_some_and(|text| !text.is_empty())
    {
        return Err(LlmError::Refusal);
    }
    let finish = choice
        .get("finish_reason")
        .and_then(Value::as_str)
        .map_or(FinishReason::Unspecified, map_finish);
    if finish == FinishReason::Refusal {
        return Err(LlmError::Refusal);
    }
    let tool_arguments = matches!(format, Format::ForcedTool(_))
        .then(|| {
            message
                .get("tool_calls")
                .and_then(|calls| calls.get(0))
                .and_then(|call| call.get("function"))
                .and_then(|function| function.get("arguments"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .flatten();
    let text = tool_arguments
        .unwrap_or_else(|| message.get("content").map(content_text).unwrap_or_default());
    Ok(Completion {
        text,
        finish,
        usage: reply.get("usage").and_then(parse_usage),
        rate_limit: RateLimit::default(),
    })
}

#[async_trait]
impl ProtocolAdapter for OpenAiChat {
    fn protocol(&self) -> Protocol {
        Protocol::OpenAiChat
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
        let response = self
            .send(&completion_request, true, &budget, cancel)
            .await?;
        let bytes = response
            .bytes_stream()
            .map(|chunk| chunk.map_err(|e| map_reqwest_error(&e)))
            .boxed();
        Ok(sse_text_stream(
            bytes,
            OpenAiDecoder::default(),
            StreamGuards {
                cancel: cancel.clone(),
                budget,
                key: self.config.key.clone(),
            },
        ))
    }

    async fn complete(
        &self,
        request: &CompletionRequest<'_>,
        cancel: &CancellationToken,
    ) -> Result<Completion, LlmError> {
        let budget = CallBudget::start(&self.config.options.background);
        let response = self.send(request, false, &budget, cancel).await?;
        let rate_limit = rate_limit_from_headers(response.headers());
        let body = read_reply(response, &budget, cancel).await?;
        let mut completion = parse_completion(
            &body,
            &request.format,
            self.config.key.as_ref().map(crate::key::ApiKey::expose),
        )?;
        completion.rate_limit = rate_limit;
        Ok(completion)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temperature_keeps_the_written_number() {
        assert_eq!(
            serde_json::to_string(&temperature_value(0.7)).expect("json"),
            "0.7"
        );
        assert_eq!(
            serde_json::to_string(&temperature_value(0.0)).expect("json"),
            "0.0"
        );
    }
}
