//! The pure half of the LLM client (PROMPT_CONTRACTS section 3): request bodies
//! and stream parsing for the two protocols. It performs no I/O. The HTTP
//! transport, retries and the host allowlist sit on top of it (S3-04).
#![forbid(unsafe_code)]

pub mod anthropic;
mod error;
mod event;
pub mod openai;
mod request;
mod sse;

pub use error::ParseError;
pub use event::{FinishReason, StreamEvent, Usage};
pub use request::{Message, Role, TextRequest};
pub use sse::{SseDecoder, SseEvent};
