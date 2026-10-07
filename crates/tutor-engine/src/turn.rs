//! One streamed tutor reply (call type T1): the stream goes in, the chunker
//! cuts it into sentences, and the session state machine is moved along the
//! way.
//!
//! The driver reports what happened and leaves the wording to its caller: an
//! empty reply completes the turn with [`ReplyOutcome::Empty`]. A call that
//! fails, or a stream that breaks before any text, moves the session to
//! `ProviderUnavailable`: there is nothing to keep and the client's own retry
//! already ran.

use crate::chunker::SentenceChunker;
use crate::event::UiEvent;
use crate::llm::LlmClient;
use crate::session::{Event, Phase, Session, TransitionError};
use llm_client::{LlmError, StreamEvent, TextRequest};
use tokio_util::sync::CancellationToken;

/// How one reply ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyOutcome {
    /// The model's reply, in full.
    Normal,
    /// The stream ended without any text.
    Empty,
    /// The learner stopped the reply. What had arrived is kept.
    Stopped,
    /// The stream broke after some text; what arrived is kept.
    Truncated,
    /// The call failed, or the stream broke before any text. The session is
    /// now `ProviderUnavailable`.
    ProviderUnavailable,
}

/// What one reply produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyReport {
    /// The reply text, as streamed.
    pub text: String,
    pub outcome: ReplyOutcome,
    /// Complete sentences, in the order they were emitted.
    pub sentences: usize,
}

/// Drives one reply on `session`, which must be in `Thinking`.
///
/// Events are emitted in the order the UI sees them: `TurnState` when the
/// reply starts and when it returns to idle, one `TutorTextDelta` per streamed
/// piece, one `TutorSentenceSpoken` per complete sentence, and `SessionState`
/// when the session changes phase (paused, provider unavailable).
///
/// A cancelled token stops the reply: text that arrived is kept, the session
/// goes back to idle when the reply had started, and to `Paused` when nothing
/// had been spoken yet.
pub async fn run_reply(
    session: &mut Session,
    client: &dyn LlmClient,
    request: TextRequest,
    cancel: &CancellationToken,
    emit: &mut dyn FnMut(UiEvent),
) -> Result<ReplyReport, TransitionError> {
    let opened = tokio::select! {
        biased;
        () = cancel.cancelled() => None,
        result = client.stream_text(request, cancel.clone()) => Some(result),
    };
    let mut stream = match opened {
        // The token fired before the call started: nothing to keep.
        None | Some(Err(LlmError::Cancelled)) => return stopped(session, false, "", 0, emit),
        Some(Err(_)) => {
            provider_failed(session, emit)?;
            return Ok(ReplyReport {
                text: String::new(),
                outcome: ReplyOutcome::ProviderUnavailable,
                sentences: 0,
            });
        }
        Some(Ok(stream)) => stream,
    };

    let mut chunker = SentenceChunker::default();
    let mut text = String::new();
    let mut sentences = 0u32;
    let mut started = false;
    let mut cancelled = false;
    let mut broke = false;

    loop {
        let item = tokio::select! {
            biased;
            () = cancel.cancelled() => { cancelled = true; break; }
            item = stream.recv() => item,
        };
        match item {
            None => break,
            Some(Ok(StreamEvent::Text(delta))) => {
                if delta.is_empty() {
                    continue;
                }
                if !started {
                    let phase = session.apply(Event::ReplyStarted)?;
                    if let Phase::Active { turn } = phase {
                        emit(UiEvent::turn_state(turn));
                    }
                    started = true;
                }
                emit(UiEvent::TutorTextDelta {
                    delta: delta.clone(),
                });
                text.push_str(&delta);
                for sentence in chunker.push(&delta) {
                    sentences += 1;
                    emit(UiEvent::TutorSentenceSpoken {
                        index: sentences - 1,
                        text: sentence,
                    });
                }
            }
            Some(Ok(StreamEvent::Finished(_) | StreamEvent::Usage(_))) => {}
            Some(Ok(StreamEvent::Error(_))) | Some(Err(_)) => {
                broke = true;
                break;
            }
        }
    }

    // Whatever the chunker still holds is the last sentence. On a stop, the
    // pending text was never spoken, so it is not emitted.
    if !cancelled {
        if let Some(rest) = chunker.finish() {
            sentences += 1;
            emit(UiEvent::TutorSentenceSpoken {
                index: sentences - 1,
                text: rest,
            });
        }
    }

    if cancelled {
        return stopped(session, started, &text, sentences, emit);
    }

    if text.trim().is_empty() {
        if broke {
            // Nothing to keep and the retry already ran: the provider is out.
            provider_failed(session, emit)?;
            return Ok(ReplyReport {
                text,
                outcome: ReplyOutcome::ProviderUnavailable,
                sentences: sentences as usize,
            });
        }
        let phase = session.apply(Event::ReplyFinished)?;
        if let Phase::Active { turn } = phase {
            emit(UiEvent::turn_state(turn));
        }
        return Ok(ReplyReport {
            text,
            outcome: ReplyOutcome::Empty,
            sentences: sentences as usize,
        });
    }

    let phase = session.apply(Event::ReplyFinished)?;
    if let Phase::Active { turn } = phase {
        emit(UiEvent::turn_state(turn));
    }
    Ok(ReplyReport {
        text,
        outcome: if broke {
            ReplyOutcome::Truncated
        } else {
            ReplyOutcome::Normal
        },
        sentences: sentences as usize,
    })
}

/// The learner stopped the reply: keep what arrived, return to idle when the
/// reply had started, otherwise wait in `Paused`.
fn stopped(
    session: &mut Session,
    started: bool,
    text: &str,
    sentences: u32,
    emit: &mut dyn FnMut(UiEvent),
) -> Result<ReplyReport, TransitionError> {
    let phase = session.apply(if started {
        Event::StopSpeaking
    } else {
        Event::Pause
    })?;
    match phase {
        Phase::Active { turn } => emit(UiEvent::turn_state(turn)),
        phase => emit(UiEvent::session_state(phase)),
    }
    Ok(ReplyReport {
        text: text.to_owned(),
        outcome: ReplyOutcome::Stopped,
        sentences: sentences as usize,
    })
}

/// The provider is not usable: move the session and tell the UI.
fn provider_failed(
    session: &mut Session,
    emit: &mut dyn FnMut(UiEvent),
) -> Result<(), TransitionError> {
    let phase = session.apply(Event::ProviderFailed)?;
    emit(UiEvent::session_state(phase));
    Ok(())
}
