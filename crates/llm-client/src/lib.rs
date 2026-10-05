//! LLM client for Lumingo.
//!
//! Two wire protocols (`openai_chat`, `anthropic_messages`) behind one
//! `LlmClient` trait. Every request goes through one HTTP client factory that
//! enforces the host allowlist and the HTTPS rule. Keys live in memory only and
//! no error, log line or debug print carries one.

#![forbid(unsafe_code)]

pub mod adapter;
pub mod caps;
pub mod client;
pub mod error;
pub mod http;
pub mod key;
pub mod openai;
pub mod profile;
pub mod redact;
pub mod schema;
mod sse;
mod stream;
mod transport;
pub mod types;

pub use adapter::{
    AdapterConfig, ClientOptions, Completion, CompletionRequest, Format, Limits, ProtocolAdapter,
    SchemaRef,
};
pub use caps::CapsHandle;
pub use client::LlmClient;
pub use error::{InvalidOutput, InvalidReason, LlmError, TimeoutKind, TransportKind};
pub use http::{GuardedClient, HttpClientFactory, SetupHosts, is_loopback_url};
pub use key::{ApiKey, KeyError};
pub use openai::OpenAiChat;
pub use profile::Protocol;
pub use schema::Contract;
pub use types::{
    Capabilities, ChatMessage, CollectedText, FinishReason, LadderLevel, RateLimit, Role,
    StreamEvent, StreamSummary, StructuredOutput, StructuredRequest, TextRequest, TextStream,
    TokenLimitParam, Usage,
};
