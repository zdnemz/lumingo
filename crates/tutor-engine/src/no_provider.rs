//! The client of a run that has no provider configured.

use async_trait::async_trait;
use llm_client::{
    Capabilities, LlmClient, LlmError, StructuredOutput, StructuredRequest, TextRequest, TextStream,
};
use tokio_util::sync::CancellationToken;

/// An [`LlmClient`] for a learner who has not set up a provider yet.
///
/// Every call fails at once with the same error, so the services that can work
/// without a model take their offline path: a productive response is stored as
/// `pending_llm`, a roleplay reports that the provider is unavailable. It is not
/// a fake: it never answers, and it says why.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoProvider;

const WHY: &str = "no provider is configured";

#[async_trait]
impl LlmClient for NoProvider {
    async fn stream_text(
        &self,
        _request: TextRequest,
        _cancel: CancellationToken,
    ) -> Result<TextStream, LlmError> {
        Err(LlmError::InvalidRequest(WHY.to_owned()))
    }

    async fn structured(
        &self,
        _request: StructuredRequest,
        _cancel: CancellationToken,
    ) -> Result<StructuredOutput, LlmError> {
        Err(LlmError::InvalidRequest(WHY.to_owned()))
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm_client::Contract;

    #[tokio::test]
    async fn every_call_fails_with_the_reason_and_nothing_is_invented() {
        let client = NoProvider;
        let structured = client
            .structured(
                StructuredRequest::new(Contract::RubricScore, "s", Vec::new(), 10),
                CancellationToken::new(),
            )
            .await;
        assert!(
            matches!(structured, Err(LlmError::InvalidRequest(m)) if m.contains("no provider"))
        );
        let text = client
            .stream_text(
                TextRequest::new("s", Vec::new(), 10),
                CancellationToken::new(),
            )
            .await;
        assert!(text.is_err());
        assert!(!client.capabilities().auth_ok);
    }
}
