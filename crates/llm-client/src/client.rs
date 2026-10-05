//! The trait the rest of the application depends on.

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::error::LlmError;
use crate::types::{Capabilities, StructuredOutput, StructuredRequest, TextRequest, TextStream};

#[async_trait]
pub trait LlmClient: Send + Sync {
    /// Streams plain text. The call is bound by the tutor-turn limits: connect 5 s,
    /// first token 8 s, total 30 s. Cancelling the token ends the stream with
    /// `LlmError::Cancelled`.
    async fn stream_text(
        &self,
        request: TextRequest,
        cancel: CancellationToken,
    ) -> Result<TextStream, LlmError>;

    /// Returns JSON that already passed local schema validation, plus how it was
    /// obtained. A reply that stays invalid after one repair is
    /// `LlmError::InvalidOutput`.
    async fn structured(
        &self,
        request: StructuredRequest,
        cancel: CancellationToken,
    ) -> Result<StructuredOutput, LlmError>;

    /// A snapshot of what is known about the provider. It is a value, not a
    /// reference, because adapters learn quirks while they work.
    fn capabilities(&self) -> Capabilities;
}
