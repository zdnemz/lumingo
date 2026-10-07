//! Structured output (PROMPT_CONTRACTS sections 3 and 5): the four-level ladder's
//! request bodies, reading the answer out of a reply, extracting the first JSON
//! value from messy text, and validating it against a schema. All pure.

use crate::{Message, Protocol, Role, openai::TokenParam};
use serde_json::{Value, json};

/// How the schema is conveyed to the model, best first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    /// Native JSON schema (`response_format` or `output_config.format`).
    NativeSchema = 1,
    /// A forced tool call whose input is the schema.
    ForcedTool = 2,
    /// `openai_chat` JSON mode, schema text in the prompt.
    JsonMode = 3,
    /// Prompt only.
    PromptOnly = 4,
}

impl Level {
    pub fn number(self) -> u8 {
        self as u8
    }

    pub fn from_number(n: u8) -> Option<Self> {
        [
            Self::NativeSchema,
            Self::ForcedTool,
            Self::JsonMode,
            Self::PromptOnly,
        ]
        .into_iter()
        .find(|l| l.number() == n)
    }

    /// The next level to try when this one fails. JSON mode only exists for `openai_chat`.
    pub fn next(self, protocol: Protocol) -> Option<Self> {
        match (self, protocol) {
            (Self::NativeSchema, _) => Some(Self::ForcedTool),
            (Self::ForcedTool, Protocol::OpenaiChat) => Some(Self::JsonMode),
            (Self::ForcedTool, Protocol::AnthropicMessages) | (Self::JsonMode, _) => {
                Some(Self::PromptOnly)
            }
            (Self::PromptOnly, _) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructuredRequest {
    pub system: Option<String>,
    pub messages: Vec<Message>,
    /// Used as the schema and tool name; letters, digits and underscores.
    pub schema_name: String,
    pub schema: Value,
    pub max_tokens: u32,
    /// Sampling temperature. `Some(0.0)` for analysis calls
    /// (`PROMPT_CONTRACTS.md` section 8). Omitted when the provider rejected it.
    pub temperature: Option<f32>,
}

impl StructuredRequest {
    /// The system text with the schema spelled out, for the levels that need it in the prompt.
    fn system_with_schema(&self) -> String {
        let instruction = format!("Return only JSON matching this schema:\n{}", self.schema);
        match &self.system {
            Some(s) => format!("{s}\n\n{instruction}"),
            None => instruction,
        }
    }

    /// The request for one repair attempt: the original conversation, the invalid
    /// output as the assistant's turn, and what was wrong.
    pub fn repair(&self, invalid_output: &str, errors: &[String]) -> Self {
        let mut messages = self.messages.clone();
        messages.push(Message {
            role: Role::Assistant,
            content: invalid_output.to_owned(),
        });
        messages.push(Message {
            role: Role::User,
            content: format!(
                "That JSON was not valid. Problems:\n- {}\nReturn corrected JSON only.",
                errors.join("\n- ")
            ),
        });
        Self {
            messages,
            ..self.clone()
        }
    }
}

fn chat_messages(system: Option<&str>, messages: &[Message]) -> Vec<Value> {
    system
        .map(|s| json!({ "role": "system", "content": s }))
        .into_iter()
        .chain(
            messages
                .iter()
                .map(|m| json!({ "role": m.role.as_str(), "content": m.content })),
        )
        .collect()
}

/// Non-streaming `openai_chat` body for `level`.
pub fn openai_body(
    req: &StructuredRequest,
    level: Level,
    token_param: TokenParam,
    model: &str,
) -> Value {
    let in_prompt = matches!(level, Level::JsonMode | Level::PromptOnly);
    let system = if in_prompt {
        Some(req.system_with_schema())
    } else {
        req.system.clone()
    };
    let mut body = json!({
        "model": model,
        "messages": chat_messages(system.as_deref(), &req.messages),
        "stream": false,
    });
    body[token_param.key()] = json!(req.max_tokens);
    if let Some(temperature) = req.temperature {
        body["temperature"] = json!(temperature);
    }
    match level {
        Level::NativeSchema => {
            body["response_format"] = json!({
                "type": "json_schema",
                "json_schema": { "name": req.schema_name, "strict": true, "schema": req.schema },
            });
        }
        Level::ForcedTool => {
            body["tools"] = json!([{ "type": "function", "function": { "name": req.schema_name, "parameters": req.schema } }]);
            body["tool_choice"] =
                json!({ "type": "function", "function": { "name": req.schema_name } });
        }
        Level::JsonMode => body["response_format"] = json!({ "type": "json_object" }),
        Level::PromptOnly => {}
    }
    body
}

/// Non-streaming `anthropic_messages` body for `level`. `JsonMode` does not exist
/// there and is treated as prompt only.
pub fn anthropic_body(req: &StructuredRequest, level: Level, model: &str) -> Value {
    let in_prompt = matches!(level, Level::JsonMode | Level::PromptOnly);
    let mut body = json!({
        "model": model,
        "max_tokens": req.max_tokens,
        "messages": req.messages.iter().map(|m| json!({ "role": m.role.as_str(), "content": m.content })).collect::<Vec<_>>(),
    });
    if let Some(temperature) = req.temperature {
        body["temperature"] = json!(temperature);
    }
    let system = if in_prompt {
        Some(req.system_with_schema())
    } else {
        req.system.clone()
    };
    if let Some(s) = system {
        body["system"] = json!(s);
    }
    match level {
        Level::NativeSchema => {
            body["output_config"] =
                json!({ "format": { "type": "json_schema", "schema": req.schema } })
        }
        Level::ForcedTool => {
            body["tools"] = json!([{ "name": req.schema_name, "input_schema": req.schema }]);
            body["tool_choice"] = json!({ "type": "tool", "name": req.schema_name });
        }
        Level::JsonMode | Level::PromptOnly => {}
    }
    body
}

/// The text that should hold the JSON, taken from a non-streaming reply.
pub fn output_text(protocol: Protocol, level: Level, reply: &Value) -> Option<String> {
    match protocol {
        Protocol::OpenaiChat => {
            let message = reply.pointer("/choices/0/message")?;
            if level == Level::ForcedTool {
                message
                    .pointer("/tool_calls/0/function/arguments")?
                    .as_str()
                    .map(str::to_owned)
            } else {
                message.get("content")?.as_str().map(str::to_owned)
            }
        }
        Protocol::AnthropicMessages => {
            let blocks = reply.get("content")?.as_array()?;
            if level == Level::ForcedTool {
                blocks
                    .iter()
                    .find(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
                    .and_then(|b| b.get("input"))
                    .map(Value::to_string)
            } else {
                let text: String = blocks
                    .iter()
                    .filter_map(|b| b.get("text").and_then(Value::as_str))
                    .collect();
                (!text.is_empty()).then_some(text)
            }
        }
    }
}

/// The first complete JSON object or array in `text`, ignoring code fences and
/// any prose around it. `None` when there is none or it does not parse.
pub fn extract_json(text: &str) -> Option<Value> {
    let start = text.find(['{', '['])?;
    let bytes = text.as_bytes();
    let (mut depth, mut in_string, mut escaped) = (0usize, false, false);
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if in_string {
            match (escaped, b) {
                (true, _) => escaped = false,
                (false, b'\\') => escaped = true,
                (false, b'"') => in_string = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' | b'[' => depth += 1,
            b'}' | b']' => {
                depth -= 1;
                if depth == 0 {
                    return serde_json::from_str(&text[start..=i]).ok();
                }
            }
            _ => {}
        }
    }
    None
}

/// Validates `instance` against `schema`. The error strings are short and may
/// quote the model's output, so they belong in a repair prompt, never in a log at info or above.
pub fn validate(schema: &Value, instance: &Value) -> Result<(), Vec<String>> {
    let validator = jsonschema::validator_for(schema)
        .map_err(|e| vec![format!("the schema itself is invalid: {e}")])?;
    let errors: Vec<String> = validator
        .iter_errors(instance)
        .map(|e| e.to_string())
        .take(8)
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> Value {
        json!({
            "type": "object",
            "properties": { "word": { "type": "string" }, "count": { "type": "integer" } },
            "required": ["word", "count"],
            "additionalProperties": false
        })
    }

    fn req() -> StructuredRequest {
        StructuredRequest {
            system: Some("You analyse.".into()),
            messages: vec![Message {
                role: Role::User,
                content: "Go".into(),
            }],
            schema_name: "test_schema".into(),
            schema: schema(),
            max_tokens: 100,
            temperature: None,
        }
    }

    #[test]
    fn levels_step_down_and_skip_json_mode_for_anthropic() {
        let chain = |p| {
            std::iter::successors(Some(Level::NativeSchema), |l| l.next(p))
                .map(Level::number)
                .collect::<Vec<_>>()
        };
        assert_eq!(chain(Protocol::OpenaiChat), [1, 2, 3, 4]);
        assert_eq!(chain(Protocol::AnthropicMessages), [1, 2, 4]);
        assert_eq!(Level::from_number(3), Some(Level::JsonMode));
        assert_eq!(Level::from_number(9), None);
    }

    #[test]
    fn openai_bodies_match_the_contract_for_every_level() {
        let b = openai_body(&req(), Level::NativeSchema, TokenParam::MaxTokens, "m");
        assert_eq!(b["response_format"]["json_schema"]["strict"], true);
        assert_eq!(b["response_format"]["json_schema"]["name"], "test_schema");
        assert_eq!(b["stream"], false);
        assert_eq!(b["max_tokens"], 100);
        let b = openai_body(
            &req(),
            Level::ForcedTool,
            TokenParam::MaxCompletionTokens,
            "m",
        );
        assert_eq!(b["tool_choice"]["function"]["name"], "test_schema");
        assert_eq!(b["tools"][0]["function"]["parameters"]["type"], "object");
        assert_eq!(b["max_completion_tokens"], 100);
        let b = openai_body(&req(), Level::JsonMode, TokenParam::MaxTokens, "m");
        assert_eq!(b["response_format"]["type"], "json_object");
        assert!(
            b["messages"][0]["content"].as_str().is_some_and(
                |s| s.contains("You analyse.") && s.contains("\"additionalProperties\"")
            )
        );
        let b = openai_body(&req(), Level::PromptOnly, TokenParam::MaxTokens, "m");
        assert!(b.get("response_format").is_none() && b.get("tools").is_none());
        assert!(
            b["messages"][0]["content"]
                .as_str()
                .is_some_and(|s| s.contains("Return only JSON"))
        );
    }

    #[test]
    fn anthropic_bodies_match_the_contract_for_every_level() {
        let b = anthropic_body(&req(), Level::NativeSchema, "m");
        assert_eq!(b["output_config"]["format"]["type"], "json_schema");
        assert_eq!(b["system"], "You analyse.");
        let b = anthropic_body(&req(), Level::ForcedTool, "m");
        assert_eq!(
            b["tool_choice"],
            json!({ "type": "tool", "name": "test_schema" })
        );
        assert_eq!(b["tools"][0]["input_schema"]["type"], "object");
        let b = anthropic_body(&req(), Level::PromptOnly, "m");
        assert!(
            b["system"]
                .as_str()
                .is_some_and(|s| s.contains("Return only JSON"))
        );
        assert!(b.get("output_config").is_none() && b.get("tools").is_none());
    }

    #[test]
    fn temperature_is_sent_when_set_and_omitted_when_none() {
        let mut req = req();
        req.temperature = Some(0.0);
        let b = openai_body(&req, Level::NativeSchema, TokenParam::MaxTokens, "m");
        assert_eq!(b["temperature"], 0.0);
        let b = anthropic_body(&req, Level::NativeSchema, "m");
        assert_eq!(b["temperature"], 0.0);
        req.temperature = None;
        let b = openai_body(&req, Level::NativeSchema, TokenParam::MaxTokens, "m");
        assert!(b.get("temperature").is_none());
        let b = anthropic_body(&req, Level::NativeSchema, "m");
        assert!(b.get("temperature").is_none());
    }

    #[test]
    fn the_answer_is_read_from_the_right_place_per_protocol_and_level() {
        let chat = json!({ "choices": [{ "message": { "content": "{\"a\":1}", "tool_calls": [{ "function": { "arguments": "{\"b\":2}" } }] } }] });
        assert_eq!(
            output_text(Protocol::OpenaiChat, Level::NativeSchema, &chat).as_deref(),
            Some("{\"a\":1}")
        );
        assert_eq!(
            output_text(Protocol::OpenaiChat, Level::ForcedTool, &chat).as_deref(),
            Some("{\"b\":2}")
        );
        let msg = json!({ "content": [{ "type": "text", "text": "{\"a\":" }, { "type": "text", "text": "1}" }, { "type": "tool_use", "input": { "b": 2 } }] });
        assert_eq!(
            output_text(Protocol::AnthropicMessages, Level::NativeSchema, &msg).as_deref(),
            Some("{\"a\":1}")
        );
        assert_eq!(
            output_text(Protocol::AnthropicMessages, Level::ForcedTool, &msg).as_deref(),
            Some("{\"b\":2}")
        );
        assert_eq!(
            output_text(Protocol::OpenaiChat, Level::NativeSchema, &json!({})),
            None
        );
        let null_content = json!({ "choices": [{ "message": { "content": null } }] });
        assert_eq!(
            output_text(Protocol::OpenaiChat, Level::NativeSchema, &null_content),
            None
        );
    }

    #[test]
    fn extract_json_finds_the_first_value_through_fences_prose_and_braces_in_strings() {
        let fenced =
            "Sure!\n```json\n{\"word\": \"a}b\", \"count\": 2}\n```\nHope that helps {not json}";
        assert_eq!(
            extract_json(fenced),
            Some(json!({ "word": "a}b", "count": 2 }))
        );
        assert_eq!(
            extract_json("[1, 2, {\"x\": [3]}] tail"),
            Some(json!([1, 2, { "x": [3] }]))
        );
        assert_eq!(
            extract_json("{\"q\": \"say \\\"hi\\\" }\"} x"),
            Some(json!({ "q": "say \"hi\" }" }))
        );
        assert_eq!(extract_json("no json here"), None);
        assert_eq!(extract_json("{\"cut\": \"off"), None);
        assert_eq!(extract_json("{not: valid}"), None);
    }

    #[test]
    fn validate_accepts_a_match_and_lists_what_is_wrong() {
        assert!(validate(&schema(), &json!({ "word": "x", "count": 1 })).is_ok());
        let errs = validate(&schema(), &json!({ "word": 5, "extra": true })).unwrap_err();
        assert!(errs.len() >= 2, "{errs:?}");
        let bad_schema = validate(&json!({ "type": 12 }), &json!({})).unwrap_err();
        assert!(bad_schema[0].contains("schema"));
    }

    #[test]
    fn a_repair_request_quotes_the_bad_output_and_the_errors_after_the_original_turns() {
        let r = req().repair("{\"word\": 5}", &["word is not a string".into()]);
        assert_eq!(r.messages.len(), 3);
        assert_eq!(
            (r.messages[1].role, r.messages[1].content.as_str()),
            (Role::Assistant, "{\"word\": 5}")
        );
        assert!(
            r.messages[2].content.contains("word is not a string")
                && r.messages[2].content.contains("corrected JSON only")
        );
        assert_eq!(r.schema, schema());
    }
}
