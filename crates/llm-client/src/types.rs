//! Request, response and capability types shared by both protocol adapters.

use std::fmt;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use futures_util::Stream;
use futures_util::StreamExt;
use futures_util::stream::BoxStream;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::LlmError;
use crate::schema::Contract;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

/// One conversation message. The system prompt is not a message: both protocols
/// receive it in their own envelope (`messages[0]` for `openai_chat`, the
/// `system` field for `anthropic_messages`), so the prompt text is the same.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
}

impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

/// A streaming plain-text call (tutor turn, explain).
#[derive(Debug, Clone, PartialEq)]
pub struct TextRequest {
    pub system: String,
    pub messages: Vec<ChatMessage>,
    pub max_tokens: u32,
    pub temperature: Option<f32>,
}

impl TextRequest {
    pub fn new(system: impl Into<String>, messages: Vec<ChatMessage>, max_tokens: u32) -> Self {
        Self {
            system: system.into(),
            messages,
            max_tokens,
            temperature: None,
        }
    }

    #[must_use]
    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }
}

/// A non-streaming call whose reply must match one of the contract schemas.
#[derive(Debug, Clone, PartialEq)]
pub struct StructuredRequest {
    pub contract: Contract,
    pub system: String,
    pub messages: Vec<ChatMessage>,
    pub max_tokens: u32,
    pub temperature: Option<f32>,
}

impl StructuredRequest {
    pub fn new(
        contract: Contract,
        system: impl Into<String>,
        messages: Vec<ChatMessage>,
        max_tokens: u32,
    ) -> Self {
        Self {
            contract,
            system: system.into(),
            messages,
            max_tokens,
            temperature: None,
        }
    }

    #[must_use]
    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }
}

/// Why the model stopped. `Refusal` covers Anthropic's `refusal` stop reason,
/// OpenAI's `content_filter` and a `refusal` delta; `Length` covers `max_tokens`
/// and `length`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinishReason {
    Stop,
    Length,
    Refusal,
    ToolCalls,
    /// The stream closed cleanly without saying why (allowed for `openai_chat`).
    Unspecified,
    Other(String),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
}

/// How a stream ended.
#[derive(Debug, Clone, PartialEq)]
pub struct StreamSummary {
    pub finish: FinishReason,
    pub usage: Option<Usage>,
    /// Time from the start of the call to the first non-empty content token.
    pub time_to_first_token: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    /// A non-empty piece of reply text.
    Delta(String),
    /// Always the last event of a stream that did not fail.
    Finished(StreamSummary),
}

/// What `collect_text` returns.
#[derive(Debug, Clone, PartialEq)]
pub struct CollectedText {
    pub text: String,
    pub summary: StreamSummary,
}

/// Streamed tutor text. Dropping it closes the connection. After an error item
/// the stream ends.
pub struct TextStream {
    inner: BoxStream<'static, Result<StreamEvent, LlmError>>,
}

impl TextStream {
    /// Wraps any stream of events. Used by the adapters and by fake clients in tests of other crates.
    pub fn new(inner: impl Stream<Item = Result<StreamEvent, LlmError>> + Send + 'static) -> Self {
        Self {
            inner: inner.boxed(),
        }
    }

    /// Reads the stream to its end. Fails with the stream's error, if any;
    /// text received before the failure is dropped, so callers that need partial
    /// text must poll the stream themselves.
    pub async fn collect_text(mut self) -> Result<CollectedText, LlmError> {
        let mut text = String::new();
        while let Some(item) = self.next().await {
            match item? {
                StreamEvent::Delta(delta) => text.push_str(&delta),
                StreamEvent::Finished(summary) => return Ok(CollectedText { text, summary }),
            }
        }
        Err(LlmError::Protocol(
            "the stream ended without a summary".to_owned(),
        ))
    }
}

impl fmt::Debug for TextStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TextStream")
    }
}

impl Stream for TextStream {
    type Item = Result<StreamEvent, LlmError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}

/// Structured-output ladder level (`docs/PROMPT_CONTRACTS.md` section 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub enum LadderLevel {
    /// Native JSON schema.
    NativeSchema,
    /// Forced tool call with the schema as tool input.
    ForcedTool,
    /// JSON mode with the schema text in the prompt (`openai_chat` only).
    JsonMode,
    /// Prompt only.
    PromptOnly,
}

impl LadderLevel {
    pub const ALL: [LadderLevel; 4] = [
        Self::NativeSchema,
        Self::ForcedTool,
        Self::JsonMode,
        Self::PromptOnly,
    ];

    pub fn as_u8(self) -> u8 {
        match self {
            Self::NativeSchema => 1,
            Self::ForcedTool => 2,
            Self::JsonMode => 3,
            Self::PromptOnly => 4,
        }
    }

    /// The next level to try after this one failed, skipping levels the protocol lacks.
    pub fn next(self, protocol: crate::profile::Protocol) -> Option<LadderLevel> {
        let mut level = self;
        loop {
            level = match level {
                Self::NativeSchema => Self::ForcedTool,
                Self::ForcedTool => Self::JsonMode,
                Self::JsonMode => Self::PromptOnly,
                Self::PromptOnly => return None,
            };
            if level != Self::JsonMode || protocol == crate::profile::Protocol::OpenAiChat {
                return Some(level);
            }
        }
    }
}

impl From<LadderLevel> for u8 {
    fn from(level: LadderLevel) -> u8 {
        level.as_u8()
    }
}

impl TryFrom<u8> for LadderLevel {
    type Error = String;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::NativeSchema),
            2 => Ok(Self::ForcedTool),
            3 => Ok(Self::JsonMode),
            4 => Ok(Self::PromptOnly),
            other => Err(format!("ladder level must be 1 to 4, got {other}")),
        }
    }
}

/// A reply that passed local validation, with how it was obtained.
#[derive(Debug, Clone, PartialEq)]
pub struct StructuredOutput {
    pub value: Value,
    pub ladder_level: LadderLevel,
    /// True when the first reply failed validation and the repair call fixed it.
    pub repaired: bool,
    /// Provider requests made for this output: the ones a provider rejected on the
    /// way down the ladder, a retry with a larger limit, and a repair.
    pub calls: u8,
    /// Tokens summed over those requests, when the provider reports them.
    pub usage: Option<Usage>,
}

/// The name of the token limit parameter an `openai_chat` server accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenLimitParam {
    MaxTokens,
    MaxCompletionTokens,
}

impl TokenLimitParam {
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::MaxTokens => "max_tokens",
            Self::MaxCompletionTokens => "max_completion_tokens",
        }
    }

    pub(crate) fn other(self) -> Self {
        match self {
            Self::MaxTokens => Self::MaxCompletionTokens,
            Self::MaxCompletionTokens => Self::MaxTokens,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateLimit {
    pub rpm: Option<u32>,
    pub rpd: Option<u32>,
}

/// What the connection test learned about a provider
/// (`docs/PROMPT_CONTRACTS.md` section 4). Serialises to the JSON stored in
/// `provider_profiles.capabilities_json`. Before the probe runs, the defaults are
/// the optimistic first guesses: the quirk fields start as "accepted" and are
/// switched off when the provider rejects them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Capabilities {
    pub probe_version: u32,
    pub auth_ok: bool,
    pub stream_ok: bool,
    pub ttft_ms: Option<u64>,
    pub tokens_per_second: Option<f64>,
    pub token_limit_param: TokenLimitParam,
    pub supports_temperature: bool,
    pub supports_usage_in_stream: bool,
    pub structured_level: Option<LadderLevel>,
    pub contracts_ok: Vec<String>,
    pub rate_limit: RateLimit,
}

impl Capabilities {
    /// Bumped when the probe steps change, so stored results can be re-run.
    pub const PROBE_VERSION: u32 = 1;
}

impl Default for Capabilities {
    fn default() -> Self {
        Self {
            probe_version: 0,
            auth_ok: false,
            stream_ok: false,
            ttft_ms: None,
            tokens_per_second: None,
            token_limit_param: TokenLimitParam::MaxTokens,
            supports_temperature: true,
            supports_usage_in_stream: true,
            structured_level: None,
            contracts_ok: Vec::new(),
            rate_limit: RateLimit::default(),
        }
    }
}
