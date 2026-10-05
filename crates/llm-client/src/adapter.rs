//! The seam between the protocol-independent logic (probe, ladder, validation)
//! and the two wire formats.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::Url;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::caps::CapsHandle;
use crate::error::LlmError;
use crate::http::GuardedClient;
use crate::key::ApiKey;
use crate::profile::Protocol;
use crate::types::{ChatMessage, FinishReason, RateLimit, TextRequest, TextStream, Usage};

/// The three time limits of `context_pack.md` section 13.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Time allowed to open the connection.
    pub connect: Duration,
    /// Time from the start of the call to the first content token. `None` for
    /// calls that do not stream.
    pub first_token: Option<Duration>,
    /// Time allowed for the whole call, retries included.
    pub total: Duration,
}

impl Limits {
    /// A tutor turn: connect 5 s, first token 8 s, total 30 s.
    pub const TUTOR_TURN: Limits = Limits {
        connect: Duration::from_secs(5),
        first_token: Some(Duration::from_secs(8)),
        total: Duration::from_secs(30),
    };

    /// Structured and probe calls do not stream and are not latency critical.
    /// `context_pack.md` section 13 names no figure for them. 60 s leaves room for a
    /// provider to compile a schema on the first request (`PROMPT_CONTRACTS.md` 3.2).
    pub const BACKGROUND: Limits = Limits {
        connect: Duration::from_secs(5),
        first_token: None,
        total: Duration::from_secs(60),
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientOptions {
    pub tutor: Limits,
    pub background: Limits,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            tutor: Limits::TUTOR_TURN,
            background: Limits::BACKGROUND,
        }
    }
}

/// What an adapter needs to talk to one provider.
#[derive(Debug, Clone)]
pub struct AdapterConfig {
    /// The base URL exactly as configured. The adapter appends its own path.
    pub base_url: Url,
    pub model: String,
    pub key: Option<ApiKey>,
    pub http: GuardedClient,
    pub caps: CapsHandle,
    pub options: ClientOptions,
}

/// A schema as it is sent to the provider.
#[derive(Debug, Clone, Copy)]
pub struct SchemaRef<'a> {
    pub name: &'a str,
    pub schema: &'a Value,
}

/// The envelope-level mechanism for one non-streaming call. Ladder levels 3 and 4
/// put the schema text in the prompt, which the ladder does before it calls an adapter.
#[derive(Debug, Clone, Copy)]
pub enum Format<'a> {
    Plain,
    /// Ladder level 1.
    NativeSchema(SchemaRef<'a>),
    /// Ladder level 2.
    ForcedTool(SchemaRef<'a>),
    /// Ladder level 3. `openai_chat` only.
    JsonMode,
}

#[derive(Debug, Clone, Copy)]
pub struct CompletionRequest<'a> {
    pub system: &'a str,
    pub messages: &'a [ChatMessage],
    pub max_tokens: u32,
    pub temperature: Option<f32>,
    pub format: Format<'a>,
}

/// The reply of a non-streaming call. `text` is the message text, or the JSON
/// arguments of the forced tool call.
#[derive(Debug, Clone, PartialEq)]
pub struct Completion {
    pub text: String,
    pub finish: FinishReason,
    pub usage: Option<Usage>,
    pub rate_limit: RateLimit,
}

#[async_trait]
pub trait ProtocolAdapter: Send + Sync {
    fn protocol(&self) -> Protocol;

    /// Streams plain text with the tutor-turn limits.
    async fn stream_text(
        &self,
        request: &TextRequest,
        cancel: &CancellationToken,
    ) -> Result<TextStream, LlmError>;

    /// One non-streaming call with the background limits.
    async fn complete(
        &self,
        request: &CompletionRequest<'_>,
        cancel: &CancellationToken,
    ) -> Result<Completion, LlmError>;
}

/// Appends `path` to the base URL exactly as configured, so a base URL that ends
/// in `/v1` keeps it and one that does not, does not gain it.
pub(crate) fn endpoint_url(base: &Url, path: &str) -> Result<Url, LlmError> {
    let joined = format!(
        "{}/{}",
        base.as_str().trim_end_matches('/'),
        path.trim_start_matches('/')
    );
    Url::parse(&joined).map_err(|_| {
        LlmError::InvalidRequest("the base URL cannot take an endpoint path".to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_keeps_the_base_path_as_given() {
        let with_v1 = Url::parse("https://api.openai.com/v1").expect("url");
        let slash = Url::parse("https://api.openai.com/v1/").expect("url");
        let bare = Url::parse("https://api.anthropic.com").expect("url");
        assert_eq!(
            endpoint_url(&with_v1, "chat/completions")
                .expect("ok")
                .as_str(),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            endpoint_url(&slash, "chat/completions")
                .expect("ok")
                .as_str(),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            endpoint_url(&bare, "v1/messages").expect("ok").as_str(),
            "https://api.anthropic.com/v1/messages"
        );
    }
}
