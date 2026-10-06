//! LLM client for Lumingo.
//!
//! Two wire protocols (`openai_chat`, `anthropic_messages`) behind one
//! `LlmClient` trait. Every request goes through one HTTP client factory that
//! enforces the host allowlist and the HTTPS rule. Keys live in memory only and
//! no error, log line or debug print carries one.

#![forbid(unsafe_code)]

pub mod adapter;
pub mod anthropic;
pub mod caps;
pub mod client;
mod connect;
pub mod error;
#[cfg(test)]
mod fake;
pub mod http;
pub mod inspector;
pub mod key;
mod ladder;
pub mod openai;
mod probe;
pub mod profile;
pub mod redact;
pub mod schema;
mod sse;
mod stream;
mod transport;
pub mod types;
mod validate;

pub use adapter::{
    AdapterConfig, ClientOptions, Completion, CompletionRequest, Format, Limits, ProtocolAdapter,
    RetryPolicy, SchemaRef,
};
pub use anthropic::AnthropicMessages;
pub use caps::CapsHandle;
pub use client::LlmClient;
pub use error::{InvalidOutput, InvalidReason, LlmError, TimeoutKind, TransportKind};
pub use http::{GuardedClient, HttpClientFactory, SetupHosts, is_loopback_url};
pub use inspector::{
    DEFAULT_CAPACITY as PAYLOAD_LOG_CAPACITY, MAX_BODY_BYTES as PAYLOAD_MAX_BODY_BYTES,
    PayloadEntry, PayloadLog, PayloadOutcome, PayloadSnapshot,
};
pub use key::{ApiKey, KeyError};
pub use ladder::{ProviderClient, REPROBE_BELOW, VALIDITY_WINDOW};
pub use openai::OpenAiChat;
pub use profile::{
    ENV_API_KEY, ENV_BASE_URL, ENV_MODEL, ENV_PROFILE_NAME, ENV_PROTOCOL, EnvProfileLoader,
    ProfileError, ProfileInfo, ProfileSet, ProfileSource, Protocol, ProviderProfile, parse_dotenv,
};
pub use schema::Contract;
pub use types::{
    Capabilities, ChatMessage, CollectedText, FinishReason, LadderLevel, RateLimit, Role,
    StreamEvent, StreamSummary, StructuredOutput, StructuredRequest, TextRequest, TextStream,
    TokenLimitParam, Usage,
};
