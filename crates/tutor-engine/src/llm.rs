//! The seam between the tutor engine and the language model (call type T1).
//!
//! The engine needs one thing from a provider: a streamed plain-text reply. The
//! trait is defined here rather than in `llm-client` so tests can script a fake
//! without touching the transport: a fake engine may exist only in test code
//! (AGENTS.md), and the real implementation is one thin adapter below.

use llm_client::{LlmError, StreamEvent, TextRequest};
use std::future::Future;
use std::pin::Pin;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// The events of one streamed reply, exactly as `llm-client` produces them.
pub type TextStream = mpsc::Receiver<Result<StreamEvent, LlmError>>;

/// One streamed tutor reply.
pub trait LlmClient: Send + Sync {
    /// Opens the stream. Failures before the first byte (auth, rate limit, a
    /// network error after the client's own retry) come back here; a failure
    /// during the stream arrives as an `Err` item on the stream.
    fn stream_text(
        &self,
        request: TextRequest,
        cancel: CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<TextStream, LlmError>> + Send + '_>>;
}

impl LlmClient for llm_client::LlmClient {
    fn stream_text(
        &self,
        request: TextRequest,
        cancel: CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<TextStream, LlmError>> + Send + '_>> {
        Box::pin(async move { llm_client::LlmClient::stream_text(self, request, cancel).await })
    }
}
