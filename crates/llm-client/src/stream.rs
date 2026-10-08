//! Turns a stream of response bytes into `TextStream` events.
//!
//! The pipeline has no channel and no spawned task: `TextStream` polls the byte
//! stream directly, so backpressure is the TCP window and dropping the stream
//! closes the connection. Every wait races three things: the cancellation token,
//! the first-token or total deadline, and the next bytes.

use std::collections::VecDeque;
use std::time::Duration;

use futures_util::Stream;
use futures_util::StreamExt;
use futures_util::stream;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::error::{LlmError, TimeoutKind};
use crate::inspector::{Capture, PayloadOutcome};
use crate::key::ApiKey;
use crate::redact::sanitize_message;
use crate::sse::{SseEvent, SseParser};
use crate::transport::CallBudget;
use crate::types::{FinishReason, StreamEvent, StreamSummary, TextStream, Usage};

/// What a protocol decoder reports for one server-sent event.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Decoded {
    /// A piece of content text. Empty strings are ignored.
    Delta(String),
    Usage(Usage),
    Finish(FinishReason),
    /// The protocol's terminal marker (`[DONE]`, `message_stop`).
    End,
}

pub(crate) trait EventDecoder: Send + 'static {
    fn decode(&mut self, event: &SseEvent) -> Result<Vec<Decoded>, LlmError>;

    /// The connection closed without a terminal marker. `finish_seen` says whether a
    /// finish reason had arrived. Return a finish reason to use, or an error when
    /// closing here means the reply is incomplete.
    fn on_close(&mut self, finish_seen: bool) -> Result<Option<FinishReason>, LlmError>;
}

pub(crate) struct StreamGuards {
    pub cancel: CancellationToken,
    pub budget: CallBudget,
    /// Used only to remove the key from error text the provider sends in the stream.
    pub key: Option<ApiKey>,
    /// The inspector entry of the request. The stream appends the bytes it
    /// receives and ends the entry when the stream completes or fails. Dropping
    /// the stream early leaves it marked abandoned.
    pub capture: Option<Capture>,
}

struct State<B, D> {
    bytes: B,
    parser: SseParser,
    decoder: D,
    guards: StreamGuards,
    queue: VecDeque<Result<StreamEvent, LlmError>>,
    finish: Option<FinishReason>,
    usage: Option<Usage>,
    first_token: Option<Duration>,
    closed: bool,
}

pub(crate) fn sse_text_stream<B, T, D>(bytes: B, decoder: D, guards: StreamGuards) -> TextStream
where
    B: Stream<Item = Result<T, LlmError>> + Send + Unpin + 'static,
    T: AsRef<[u8]> + Send + 'static,
    D: EventDecoder,
{
    let state = State {
        bytes,
        parser: SseParser::new(),
        decoder,
        guards,
        queue: VecDeque::new(),
        finish: None,
        usage: None,
        first_token: None,
        closed: false,
    };
    TextStream::new(stream::unfold(state, |mut state| async move {
        let item = state.next_item().await?;
        Some((item, state))
    }))
}

impl<B, T, D> State<B, D>
where
    B: Stream<Item = Result<T, LlmError>> + Send + Unpin,
    T: AsRef<[u8]> + Send,
    D: EventDecoder,
{
    async fn next_item(&mut self) -> Option<Result<StreamEvent, LlmError>> {
        loop {
            if let Some(item) = self.queue.pop_front() {
                return Some(item);
            }
            if self.closed {
                return None;
            }
            self.step().await;
        }
    }

    /// The deadline the next wait is bound by. Until the first content token the
    /// first-token limit applies, then only the total limit.
    fn deadline(&self) -> (Instant, TimeoutKind) {
        let budget = &self.guards.budget;
        match (self.first_token, budget.first_token_deadline) {
            (None, Some(first)) if first < budget.total_deadline => {
                (first, TimeoutKind::FirstToken)
            }
            _ => (budget.total_deadline, TimeoutKind::Total),
        }
    }

    async fn step(&mut self) {
        enum Wake<T> {
            Cancelled,
            Deadline,
            Bytes(Option<Result<T, LlmError>>),
        }

        let (deadline, kind) = self.deadline();
        let cancel = self.guards.cancel.clone();
        // The select only produces a value; the state is touched after it, once the
        // borrow of `self.bytes` has ended.
        let wake = tokio::select! {
            biased;
            () = cancel.cancelled() => Wake::Cancelled,
            () = tokio::time::sleep_until(deadline) => Wake::Deadline,
            next = self.bytes.next() => Wake::Bytes(next),
        };
        match wake {
            Wake::Cancelled => self.fail(LlmError::Cancelled),
            Wake::Deadline => self.fail(LlmError::Timeout(kind)),
            Wake::Bytes(None) => self.on_connection_closed(),
            Wake::Bytes(Some(Err(error))) => self.fail(error),
            Wake::Bytes(Some(Ok(chunk))) => {
                if let Some(capture) = self.guards.capture.as_mut() {
                    capture.append(chunk.as_ref());
                }
                self.on_bytes(chunk.as_ref());
            }
        }
    }

    fn on_bytes(&mut self, chunk: &[u8]) {
        match self.parser.push(chunk) {
            Ok(events) => {
                for event in events {
                    if !self.handle_event(&event) {
                        break;
                    }
                }
            }
            Err(error) => self.fail(error),
        }
    }

    /// Returns false when the stream is over and later events must be ignored.
    fn handle_event(&mut self, event: &SseEvent) -> bool {
        let decoded = match self.decoder.decode(event) {
            Ok(decoded) => decoded,
            Err(error) => {
                self.fail(error);
                return false;
            }
        };
        for item in decoded {
            match item {
                Decoded::Delta(text) if text.is_empty() => {}
                Decoded::Delta(text) => {
                    if self.first_token.is_none() {
                        self.first_token = Some(self.guards.budget.started.elapsed());
                    }
                    self.queue.push_back(Ok(StreamEvent::Delta(text)));
                }
                Decoded::Usage(usage) => self.usage = Some(usage),
                Decoded::Finish(reason) => self.finish = Some(reason),
                Decoded::End => {
                    self.complete(None);
                    return false;
                }
            }
        }
        true
    }

    fn on_connection_closed(&mut self) {
        if let Some(event) = self.parser.finish() {
            if !self.handle_event(&event) {
                return;
            }
        }
        match self.decoder.on_close(self.finish.is_some()) {
            Ok(finish) => self.complete(finish),
            Err(error) => self.fail(error),
        }
    }

    fn complete(&mut self, fallback_finish: Option<FinishReason>) {
        let finish = self
            .finish
            .take()
            .or(fallback_finish)
            .unwrap_or(FinishReason::Unspecified);
        self.queue
            .push_back(Ok(StreamEvent::Finished(StreamSummary {
                finish,
                usage: self.usage,
                time_to_first_token: self.first_token,
            })));
        self.closed = true;
        if let Some(capture) = self.guards.capture.take() {
            capture.finish(PayloadOutcome::Ok);
        }
    }

    fn fail(&mut self, error: LlmError) {
        let error = match error {
            LlmError::Stream { message } => LlmError::Stream {
                message: sanitize_message(&message, self.guards.key.as_ref().map(ApiKey::expose)),
            },
            other => other,
        };
        self.queue.push_back(Err(error));
        self.closed = true;
        if let Some(capture) = self.guards.capture.take() {
            capture.finish(PayloadOutcome::Failed);
        }
    }
}
