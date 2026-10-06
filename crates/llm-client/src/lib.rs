//! The LLM client (PROMPT_CONTRACTS section 3). The adapters (request bodies and
//! stream parsing) are pure; `LlmClient` adds the HTTP transport, retries, timeouts
//! and the host rules (S3-01, S3-04).
#![forbid(unsafe_code)]

pub mod anthropic;
mod client;
mod env;
mod error;
mod event;
mod key;
pub mod openai;
pub mod policy;
mod profiles;
mod request;
mod sse;
mod structured;

pub use client::{Capabilities, LlmClient, Protocol, ProviderConfig, StructuredOutput, Timeouts};
pub use env::{EnvError, load_env_profile, parse_dotenv, provider_from_vars};
pub use error::{LlmError, ParseError};
pub use event::{FinishReason, StreamEvent, Usage};
pub use key::ApiKey;
pub use profiles::{
    ENV_PROFILE_NAME, Profile, ProfileSummary, ProfilesError, read_profiles, write_profiles,
};
pub use request::{Message, Role, TextRequest};
pub use sse::{SseDecoder, SseEvent};
pub use structured::{Level, StructuredRequest, extract_json, validate};
