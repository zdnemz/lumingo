//! One streamed tutor reply (call type T1): the stream goes in, the chunker
//! cuts it into sentences, and the session state machine is moved along the
//! way.
//!
//! Output handling follows T1: a call that fails before any text, or a stream
//! that breaks before any text, moves the session to `ProviderUnavailable`
//! (the client already retried once). An empty or refused answer becomes the
//! authored [`FALLBACK_LINE`] once; the second time it happens the caller
//! passes `allow_fallback: false`, which moves the session to
//! `ProviderUnavailable` as well. Text that arrived before a stream error is
//! kept and reported as `Truncated`.

use crate::chunker::SentenceChunker;
use crate::event::UiEvent;
use crate::llm::LlmClient;
use crate::prompt::FALLBACK_LINE;
use crate::session::{Event, Phase, Session, TransitionError};
use llm_client::{LlmError, StreamEvent, TextRequest};
use tokio_util::sync::CancellationToken;

/// How one reply ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyOutcome {
    /// The model's reply, in full.
    Normal,
    /// The model said nothing or refused: the authored line was spoken instead.
    Fallback,
    /// The learner stopped the reply. What had arrived is kept.
    Stopped,
    /// The stream broke after some text; what arrived is kept.
    Truncated,
    /// The call failed, or an empty answer happened with no fallback left. The
    /// session is now `ProviderUnavailable`.
    ProviderUnavailable,
}

/// What one reply produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyReport {
    /// The reply text, as streamed (or the fallback line).
    pub text: String,
    pub outcome: ReplyOutcome,
    /// Complete sentences, in the order they were emitted.
    pub sentences: usize,
}

/// Drives one reply on `session`, which must be in `Thinking`. For a reply to a
/// learner turn the caller applies that turn's event first (`TextSent`, or
/// `UtteranceEnded` then `TranscriptReady`); for the tutor-first opening turn it
/// applies `Event::OpeningTurn`.
///
/// `allow_fallback` is whether the authored line may be used this time. The
/// caller passes `false` for the second empty or refused answer in a row; the
/// turn then ends as `ProviderUnavailable`.
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
    allow_fallback: bool,
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
        if !allow_fallback {
            // An empty or refused answer with no fallback left: the second time
            // this happens in a session the provider is treated as failing.
            provider_failed(session, emit)?;
            return Ok(ReplyReport {
                text,
                outcome: ReplyOutcome::ProviderUnavailable,
                sentences: sentences as usize,
            });
        }
        // An empty or refused answer: the authored line, once.
        if !started {
            let phase = session.apply(Event::ReplyStarted)?;
            if let Phase::Active { turn } = phase {
                emit(UiEvent::turn_state(turn));
            }
        }
        emit(UiEvent::TutorTextDelta {
            delta: FALLBACK_LINE.to_owned(),
        });
        emit(UiEvent::TutorSentenceSpoken {
            index: 0,
            text: FALLBACK_LINE.to_owned(),
        });
        let phase = session.apply(Event::ReplyFinished)?;
        if let Phase::Active { turn } = phase {
            emit(UiEvent::turn_state(turn));
        }
        return Ok(ReplyReport {
            text: FALLBACK_LINE.to_owned(),
            outcome: ReplyOutcome::Fallback,
            sentences: 1,
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
