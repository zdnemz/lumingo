//! The LLM client (PROMPT_CONTRACTS section 3). The adapters (request bodies and
//! stream parsing) are pure; `LlmClient` adds the HTTP transport, retries, timeouts
//! and the host rules (S3-01, S3-04).
#![forbid(unsafe_code)]

pub mod anthropic;
mod client;
mod error;
mod event;
mod key;
pub mod openai;
pub mod policy;
mod request;
mod sse;

pub use client::{LlmClient, Protocol, ProviderConfig, Timeouts};
pub use error::{LlmError, ParseError};
pub use event::{FinishReason, StreamEvent, Usage};
pub use key::ApiKey;
pub use request::{Message, Role, TextRequest};
pub use sse::{SseDecoder, SseEvent};
