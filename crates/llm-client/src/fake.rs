//! A scripted adapter for the ladder and probe tests: no network, every reply is
//! queued in advance. It exists only in test builds.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::stream;
use tokio_util::sync::CancellationToken;

use crate::adapter::{Completion, CompletionRequest, Format, ProtocolAdapter};
use crate::error::LlmError;
use crate::profile::Protocol;
use crate::types::{
    FinishReason, RateLimit, StreamEvent, StreamSummary, TextRequest, TextStream, Usage,
};

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Seen {
    Plain,
    Native,
    Tool,
    Json,
}

#[derive(Debug, Clone)]
pub(crate) struct SeenRequest {
    pub format: Seen,
    pub system: String,
    pub messages: Vec<(String, String)>,
    pub max_tokens: u32,
}

pub(crate) type Reply = Result<Completion, LlmError>;
pub(crate) type StreamReply = Result<Vec<Result<StreamEvent, LlmError>>, LlmError>;

pub(crate) struct FakeAdapter {
    protocol: Protocol,
    replies: Mutex<VecDeque<Reply>>,
    streams: Mutex<VecDeque<StreamReply>>,
    seen: Mutex<Vec<SeenRequest>>,
}

impl FakeAdapter {
    pub(crate) fn new(protocol: Protocol, replies: Vec<Reply>) -> Self {
        Self {
            protocol,
            replies: Mutex::new(replies.into()),
            streams: Mutex::new(VecDeque::new()),
            seen: Mutex::new(Vec::new()),
        }
    }

    #[must_use]
    pub(crate) fn with_streams(self, streams: Vec<StreamReply>) -> Self {
        *self.streams.lock().expect("lock") = streams.into();
        self
    }

    pub(crate) fn requests(&self) -> Vec<SeenRequest> {
        self.seen.lock().expect("lock").clone()
    }
}

fn completion(text: &str, finish: FinishReason) -> Completion {
    Completion {
        text: text.to_owned(),
        finish,
        usage: Some(Usage {
            input_tokens: Some(10),
            output_tokens: Some(5),
        }),
        rate_limit: RateLimit::default(),
    }
}

pub(crate) fn text(text: &str) -> Reply {
    Ok(completion(text, FinishReason::Stop))
}

pub(crate) fn text_with(text: &str, finish: FinishReason) -> Reply {
    Ok(completion(text, finish))
}

pub(crate) fn cut_off(text: &str) -> Reply {
    text_with(text, FinishReason::Length)
}

pub(crate) fn rejected(message: &str) -> Reply {
    Err(LlmError::Rejected {
        status: 400,
        message: message.to_owned(),
        param: None,
    })
}

/// A stream of the given chunks that ends normally, with usage.
pub(crate) fn stream_of(chunks: &[&str]) -> StreamReply {
    let mut events: Vec<Result<StreamEvent, LlmError>> = chunks
        .iter()
        .map(|c| Ok(StreamEvent::Delta((*c).to_owned())))
        .collect();
    events.push(Ok(StreamEvent::Finished(StreamSummary {
        finish: FinishReason::Stop,
        usage: Some(Usage {
            input_tokens: Some(8),
            output_tokens: Some(6),
        }),
        time_to_first_token: Some(Duration::from_millis(120)),
    })));
    Ok(events)
}

#[async_trait]
impl ProtocolAdapter for FakeAdapter {
    fn protocol(&self) -> Protocol {
        self.protocol
    }

    async fn stream_text(
        &self,
        _: &TextRequest,
        _: &CancellationToken,
    ) -> Result<TextStream, LlmError> {
        match self.streams.lock().expect("lock").pop_front() {
            Some(Ok(events)) => Ok(TextStream::new(stream::iter(events))),
            Some(Err(error)) => Err(error),
            None => Err(LlmError::InvalidRequest(
                "the fake adapter has no stream queued".to_owned(),
            )),
        }
    }

    async fn complete(
        &self,
        request: &CompletionRequest<'_>,
        _: &CancellationToken,
    ) -> Result<Completion, LlmError> {
        let format = match request.format {
            Format::Plain => Seen::Plain,
            Format::NativeSchema(_) => Seen::Native,
            Format::ForcedTool(_) => Seen::Tool,
            Format::JsonMode => Seen::Json,
        };
        self.seen.lock().expect("lock").push(SeenRequest {
            format,
            system: request.system.to_owned(),
            messages: request
                .messages
                .iter()
                .map(|m| (format!("{:?}", m.role), m.content.clone()))
                .collect(),
            max_tokens: request.max_tokens,
        });
        self.replies
            .lock()
            .expect("lock")
            .pop_front()
            .unwrap_or_else(|| {
                Err(LlmError::Protocol(
                    "the fake adapter ran out of replies".to_owned(),
                ))
            })
    }
}
