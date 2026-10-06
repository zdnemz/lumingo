//! `openai_chat` (PROMPT_CONTRACTS 3.1).

use crate::{FinishReason, ParseError, SseEvent, StreamEvent, TextRequest, Usage};
use serde_json::{Value, json};

/// Servers disagree on the token-limit parameter name. The caller tries one and
/// switches on an HTTP 400 that names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TokenParam {
    #[default]
    MaxTokens,
    MaxCompletionTokens,
}

impl TokenParam {
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::MaxTokens => "max_tokens",
            Self::MaxCompletionTokens => "max_completion_tokens",
        }
    }

    pub fn other(self) -> Self {
        match self {
            Self::MaxTokens => Self::MaxCompletionTokens,
            Self::MaxCompletionTokens => Self::MaxTokens,
        }
    }
}

/// Body for `POST {base_url}/chat/completions`, streaming. `include_usage` asks for
/// token counts and is dropped when the server rejects it.
pub fn request_body(req: &TextRequest, token_param: TokenParam, include_usage: bool) -> Value {
    let system = req
        .system
        .iter()
        .map(|s| json!({ "role": "system", "content": s }));
    let turns = req
        .messages
        .iter()
        .map(|m| json!({ "role": m.role.as_str(), "content": m.content }));
    let mut body = json!({
        "model": req.model,
        "messages": system.chain(turns).collect::<Vec<_>>(),
        "stream": true,
    });
    body[token_param.key()] = json!(req.max_tokens);
    if let Some(t) = req.temperature {
        body["temperature"] = json!(t);
    }
    if include_usage {
        body["stream_options"] = json!({ "include_usage": true });
    }
    body
}

/// Turns one SSE event into zero or more stream events. `[DONE]`, empty and null
/// deltas, and every field other than `content` (reasoning text included) yield nothing.
pub fn parse_event(event: &SseEvent) -> Result<Vec<StreamEvent>, ParseError> {
    let data = event.data.trim();
    if data == "[DONE]" {
        return Ok(Vec::new());
    }
    let v: Value = serde_json::from_str(data).map_err(|_| ParseError::InvalidJson)?;
    if let Some(err) = v.get("error") {
        let msg = err
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("provider error");
        return Ok(vec![StreamEvent::Error(msg.to_owned())]);
    }
    let mut out = Vec::new();
    if let Some(choice) = v.get("choices").and_then(|c| c.get(0)) {
        if let Some(text) = choice
            .pointer("/delta/content")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
        {
            out.push(StreamEvent::Text(text.to_owned()));
        }
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            out.push(StreamEvent::Finished(match reason {
                "stop" => FinishReason::Stop,
                "length" => FinishReason::Length,
                "content_filter" => FinishReason::Refusal,
                other => FinishReason::Other(other.to_owned()),
            }));
        }
    }
    if let Some(u) = v.get("usage").filter(|u| !u.is_null()) {
        out.push(StreamEvent::Usage(Usage {
            input_tokens: u.get("prompt_tokens").and_then(Value::as_u64),
            output_tokens: u.get("completion_tokens").and_then(Value::as_u64),
        }));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Message, Role, SseDecoder};

    fn stream(raw: &str) -> Vec<StreamEvent> {
        let mut d = SseDecoder::default();
        let mut events = d.push(raw.as_bytes());
        events.extend(d.finish());
        events
            .iter()
            .flat_map(|e| parse_event(e).unwrap())
            .collect()
    }

    fn text(s: &str) -> StreamEvent {
        StreamEvent::Text(s.to_owned())
    }

    #[test]
    fn a_normal_stream_yields_text_finish_and_usage() {
        let raw = concat!(
            ": keep-alive\n\n",
            "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\" there.\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":3}}\n\n",
            "data: [DONE]\n\n",
        );
        assert_eq!(
            stream(raw),
            [
                text("Hello"),
                text(" there."),
                StreamEvent::Finished(FinishReason::Stop),
                StreamEvent::Usage(Usage {
                    input_tokens: Some(12),
                    output_tokens: Some(3)
                }),
            ]
        );
    }

    #[test]
    fn null_content_reasoning_fields_and_a_missing_done_are_tolerated() {
        let raw = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":null,\"reasoning_content\":\"thinking\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"length\"}]}",
        );
        assert_eq!(
            stream(raw),
            [text("Hi"), StreamEvent::Finished(FinishReason::Length)]
        );
    }

    #[test]
    fn an_error_chunk_keeps_only_its_message() {
        let raw = "data: {\"error\":{\"message\":\"rate limited\",\"type\":\"x\"}}\n\n";
        assert_eq!(stream(raw), [StreamEvent::Error("rate limited".into())]);
    }

    #[test]
    fn invalid_json_is_a_typed_error_that_does_not_echo_the_payload() {
        let e = parse_event(&SseEvent {
            event: None,
            data: "sk-secret not json".into(),
        })
        .unwrap_err();
        assert_eq!(e, ParseError::InvalidJson);
        assert!(!e.to_string().contains("sk-secret"));
    }

    fn request() -> TextRequest {
        TextRequest {
            model: "m".into(),
            system: Some("Be brief.".into()),
            messages: vec![Message {
                role: Role::User,
                content: "Hi".into(),
            }],
            max_tokens: 200,
            temperature: Some(0.5),
        }
    }

    #[test]
    fn body_uses_the_requested_token_parameter_name() {
        let a = request_body(&request(), TokenParam::MaxTokens, true);
        assert_eq!(a["max_tokens"], 200);
        assert!(a.get("max_completion_tokens").is_none());
        assert_eq!(a["stream_options"]["include_usage"], true);
        assert_eq!(a["messages"][0]["role"], "system");
        let b = request_body(&request(), TokenParam::MaxTokens.other(), false);
        assert_eq!(b["max_completion_tokens"], 200);
        assert!(b.get("max_tokens").is_none() && b.get("stream_options").is_none());
    }

    #[test]
    fn temperature_and_system_are_left_out_when_absent() {
        let mut r = request();
        r.temperature = None;
        r.system = None;
        let b = request_body(&r, TokenParam::MaxTokens, false);
        assert!(b.get("temperature").is_none());
        assert_eq!(b["messages"].as_array().map(Vec::len), Some(1));
    }
}
