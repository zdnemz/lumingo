/// What the rest of the program sees, whatever the protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEvent {
    /// A piece of reply text. Never empty.
    Text(String),
    Usage(Usage),
    Finished(FinishReason),
    /// The provider reported an error inside the stream. Only its short message is kept.
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinishReason {
    Stop,
    /// The token limit was hit: speak the complete sentences received so far.
    Length,
    Refusal,
    Other(String),
}
