//! `anthropic_messages` (PROMPT_CONTRACTS 3.2).

use crate::{FinishReason, ParseError, SseEvent, StreamEvent, TextRequest, Usage};
use serde_json::{Value, json};

/// Body for `POST {base_url}/v1/messages`, streaming. `max_tokens` is required by the API.
pub fn request_body(req: &TextRequest) -> Value {
    let mut body = json!({
        "model": req.model,
        "max_tokens": req.max_tokens,
        "messages": req.messages.iter().map(|m| json!({ "role": m.role.as_str(), "content": m.content })).collect::<Vec<_>>(),
        "stream": true,
    });
    if let Some(s) = &req.system {
        body["system"] = json!(s);
    }
    if let Some(t) = req.temperature {
        body["temperature"] = json!(t);
    }
    body
}

/// Turns one SSE event into stream events. Only `text_delta` text is kept; `ping`,
/// block start and stop, and other delta types (tool input, thinking) yield nothing.
pub fn parse_event(event: &SseEvent) -> Result<Vec<StreamEvent>, ParseError> {
    let v: Value = serde_json::from_str(event.data.trim()).map_err(|_| ParseError::InvalidJson)?;
    // The `type` field in the payload matches the SSE event name; prefer it.
    let kind = v
        .get("type")
        .and_then(Value::as_str)
        .or(event.event.as_deref())
        .ok_or(ParseError::Missing("type"))?;
    let tokens = |path: &str| v.pointer(path).and_then(Value::as_u64);
    Ok(match kind {
        "message_start" => vec![StreamEvent::Usage(Usage {
            input_tokens: tokens("/message/usage/input_tokens"),
            output_tokens: tokens("/message/usage/output_tokens"),
        })],
        "content_block_delta" => match v.pointer("/delta/type").and_then(Value::as_str) {
            Some("text_delta") => v
                .pointer("/delta/text")
                .and_then(Value::as_str)
                .filter(|t| !t.is_empty())
                .map(|t| vec![StreamEvent::Text(t.to_owned())])
                .unwrap_or_default(),
            _ => Vec::new(),
        },
        "message_delta" => {
            let mut out = Vec::new();
            if let Some(output_tokens) = tokens("/usage/output_tokens") {
                out.push(StreamEvent::Usage(Usage {
                    input_tokens: tokens("/usage/input_tokens"),
                    output_tokens: Some(output_tokens),
                }));
            }
            if let Some(reason) = v.pointer("/delta/stop_reason").and_then(Value::as_str) {
                out.push(StreamEvent::Finished(match reason {
                    "end_turn" | "stop_sequence" => FinishReason::Stop,
                    "max_tokens" => FinishReason::Length,
                    "refusal" => FinishReason::Refusal,
                    other => FinishReason::Other(other.to_owned()),
                }));
            }
            out
        }
        "error" => {
            let msg = v
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("provider error");
            vec![StreamEvent::Error(msg.to_owned())]
        }
        _ => Vec::new(),
    })
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

    #[test]
    fn a_normal_stream_yields_text_usage_and_the_stop_reason() {
        let raw = concat!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":25,\"output_tokens\":1}}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: ping\ndata: {\"type\":\"ping\"}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\" there.\"}}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":15}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        );
        assert_eq!(
            stream(raw),
            [
                StreamEvent::Usage(Usage {
                    input_tokens: Some(25),
                    output_tokens: Some(1)
                }),
                StreamEvent::Text("Hello".into()),
                StreamEvent::Text(" there.".into()),
                StreamEvent::Usage(Usage {
                    input_tokens: None,
                    output_tokens: Some(15)
                }),
                StreamEvent::Finished(FinishReason::Stop),
            ]
        );
    }

    #[test]
    fn max_tokens_and_refusal_are_reported_as_such() {
        let delta = |r: &str| {
            format!(
                "data: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"{r}\"}}}}\n\n"
            )
        };
        assert_eq!(
            stream(&delta("max_tokens")),
            [StreamEvent::Finished(FinishReason::Length)]
        );
        assert_eq!(
            stream(&delta("refusal")),
            [StreamEvent::Finished(FinishReason::Refusal)]
        );
        assert_eq!(
            stream(&delta("pause_turn")),
            [StreamEvent::Finished(FinishReason::Other(
                "pause_turn".into()
            ))]
        );
    }

    #[test]
    fn tool_input_deltas_are_ignored_and_errors_keep_only_the_message() {
        let raw = concat!(
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"a\\\"\"}}\n\n",
            "event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\n",
        );
        assert_eq!(stream(raw), [StreamEvent::Error("Overloaded".into())]);
    }

    #[test]
    fn the_event_name_is_used_when_the_payload_has_no_type() {
        let e = SseEvent {
            event: Some("message_delta".into()),
            data: "{\"delta\":{\"stop_reason\":\"end_turn\"}}".into(),
        };
        assert_eq!(
            parse_event(&e),
            Ok(vec![StreamEvent::Finished(FinishReason::Stop)])
        );
        let none = SseEvent {
            event: None,
            data: "{}".into(),
        };
        assert_eq!(parse_event(&none), Err(ParseError::Missing("type")));
    }

    #[test]
    fn body_has_required_max_tokens_and_a_top_level_system() {
        let req = TextRequest {
            model: "m".into(),
            system: Some("Be brief.".into()),
            messages: vec![Message {
                role: Role::User,
                content: "Hi".into(),
            }],
            max_tokens: 200,
            temperature: None,
        };
        let b = request_body(&req);
        assert_eq!(
            (
                b["max_tokens"].clone(),
                b["system"].clone(),
                b["stream"].clone()
            ),
            (json!(200), json!("Be brief."), json!(true))
        );
        assert!(b.get("temperature").is_none());
    }
}
