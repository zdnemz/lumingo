//! The streamed tutor reply (S4-04) driven by a scripted `LlmClient`.
//! The fake exists only here, in test code, as AGENTS.md requires.
#![allow(clippy::unwrap_used)] // test helpers; clippy.toml only exempts #[test] functions

use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use std::time::Duration;

use llm_client::{LlmError, Message, Role, StreamEvent, TextRequest};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tutor_engine::{
    Channel, Event, LlmClient, Phase, ReplyOutcome, Session, SessionKind, TextStream, TurnState,
    UiEvent, run_reply,
};

/// What the fake sends on the stream.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Text(&'static str),
    Finished,
    /// A provider error inside the stream.
    StreamError,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum After {
    /// The stream ends after the scripted steps.
    Close,
    /// The stream stays open until the token is cancelled.
    Silent,
}

#[derive(Clone)]
enum Script {
    /// The call fails before the first byte.
    FailOpen,
    Steps {
        steps: Vec<Step>,
        after: After,
    },
}

/// A scripted client. Records every request it is handed.
struct ScriptedClient {
    script: Script,
    sent: Mutex<Vec<TextRequest>>,
}

impl ScriptedClient {
    fn new(script: Script) -> Self {
        Self {
            script,
            sent: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<TextRequest> {
        self.sent.lock().unwrap().clone()
    }
}

impl LlmClient for ScriptedClient {
    fn stream_text(
        &self,
        request: TextRequest,
        cancel: CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<TextStream, LlmError>> + Send + '_>> {
        self.sent.lock().unwrap().push(request);
        let script = self.script.clone();
        Box::pin(async move {
            let Script::Steps { steps, after } = script else {
                return Err(LlmError::Transport("scripted open failure".to_owned()));
            };
            let (tx, rx) = mpsc::channel(16);
            tokio::spawn(async move {
                for step in steps {
                    let event = match step {
                        Step::Text(text) => Ok(StreamEvent::Text(text.to_owned())),
                        Step::Finished => Ok(StreamEvent::Finished(llm_client::FinishReason::Stop)),
                        Step::StreamError => Err(LlmError::Transport("scripted break".to_owned())),
                    };
                    if tx.send(event).await.is_err() {
                        return;
                    }
                }
                if after == After::Silent {
                    cancel.cancelled().await;
                }
            });
            Ok(rx)
        })
    }
}

fn request() -> TextRequest {
    TextRequest {
        model: "test-model".to_owned(),
        system: Some("system".to_owned()),
        messages: vec![Message {
            role: Role::User,
            content: "Hi".to_owned(),
        }],
        max_tokens: 80,
        temperature: Some(0.7),
    }
}

/// A session in `Thinking`, ready for a reply.
fn thinking(kind: SessionKind, channel: Channel) -> Session {
    let mut session = Session::new(kind, channel);
    match channel {
        Channel::Voice => {
            session.apply(Event::UtteranceEnded).unwrap();
            session.apply(Event::TranscriptReady).unwrap();
        }
        Channel::Text => {
            session.apply(Event::TextSent).unwrap();
        }
    }
    session
}

fn voice() -> Session {
    thinking(SessionKind::Conversation, Channel::Voice)
}

fn text() -> Session {
    thinking(SessionKind::TextChat, Channel::Text)
}

async fn drive(
    session: &mut Session,
    client: &ScriptedClient,
    cancel: &CancellationToken,
    allow_fallback: bool,
) -> (tutor_engine::ReplyReport, Vec<UiEvent>) {
    let mut events = Vec::new();
    let report = run_reply(
        session,
        client,
        request(),
        cancel,
        allow_fallback,
        &mut |e| events.push(e),
    )
    .await
    .unwrap();
    (report, events)
}

#[test]
fn a_voice_reply_streams_sentences_and_returns_to_listening() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let client = ScriptedClient::new(Script::Steps {
            steps: vec![
                Step::Text("Hello there. "),
                Step::Text("How are you? Ok"),
                Step::Finished,
            ],
            after: After::Close,
        });
        let mut session = voice();
        let (report, events) = drive(&mut session, &client, &CancellationToken::new(), true).await;

        assert_eq!(report.outcome, ReplyOutcome::Normal);
        assert_eq!(report.text, "Hello there. How are you? Ok");
        assert_eq!(report.sentences, 3);
        assert_eq!(session.turn(), Some(TurnState::Listening));
        assert_eq!(session.turns_completed(), 1);

        let sentences: Vec<(u32, String)> = events
            .iter()
            .filter_map(|e| match e {
                UiEvent::TutorSentenceSpoken { index, text } => Some((*index, text.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(
            sentences,
            [
                (0, "Hello there.".to_owned()),
                (1, "How are you?".to_owned()),
                (2, "Ok".to_owned()),
            ]
        );
        assert!(events.contains(&UiEvent::turn_state(TurnState::Speaking)));
        assert!(events.contains(&UiEvent::turn_state(TurnState::Listening)));
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, UiEvent::TutorTextDelta { .. }))
                .count(),
            2
        );
    });
}

#[test]
fn a_text_reply_walks_waiting_thinking_replying_waiting() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let client = ScriptedClient::new(Script::Steps {
            steps: vec![Step::Text("Sure. Let's begin."), Step::Finished],
            after: After::Close,
        });
        let mut session = text();
        let (report, events) = drive(&mut session, &client, &CancellationToken::new(), true).await;

        assert_eq!(report.outcome, ReplyOutcome::Normal);
        assert_eq!(session.turn(), Some(TurnState::Waiting));
        assert_eq!(
            events,
            [
                UiEvent::turn_state(TurnState::Replying),
                UiEvent::TutorTextDelta {
                    delta: "Sure. Let's begin.".to_owned()
                },
                UiEvent::TutorSentenceSpoken {
                    index: 0,
                    text: "Sure.".to_owned()
                },
                UiEvent::TutorSentenceSpoken {
                    index: 1,
                    text: "Let's begin.".to_owned()
                },
                UiEvent::turn_state(TurnState::Waiting),
            ]
        );
    });
}

#[test]
fn an_empty_reply_speaks_the_authored_line_once() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let client = ScriptedClient::new(Script::Steps {
            steps: vec![Step::Finished],
            after: After::Close,
        });
        let mut session = voice();
        let (report, events) = drive(&mut session, &client, &CancellationToken::new(), true).await;

        assert_eq!(report.outcome, ReplyOutcome::Fallback);
        assert_eq!(report.text, tutor_engine::FALLBACK_LINE);
        assert_eq!(report.sentences, 1);
        assert_eq!(session.turn(), Some(TurnState::Listening));
        assert_eq!(session.turns_completed(), 1);
        assert!(events.contains(&UiEvent::TutorSentenceSpoken {
            index: 0,
            text: tutor_engine::FALLBACK_LINE.to_owned()
        }));
        assert_eq!(
            events.last(),
            Some(&UiEvent::turn_state(TurnState::Listening))
        );
    });
}

#[test]
fn a_second_empty_reply_without_fallback_moves_to_provider_unavailable() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let client = ScriptedClient::new(Script::Steps {
            steps: vec![Step::Finished],
            after: After::Close,
        });
        let mut session = voice();
        let (report, events) = drive(&mut session, &client, &CancellationToken::new(), false).await;

        assert_eq!(report.outcome, ReplyOutcome::ProviderUnavailable);
        assert!(report.text.is_empty());
        assert_eq!(session.phase(), Phase::ProviderUnavailable);
        assert_eq!(events, [UiEvent::session_state(Phase::ProviderUnavailable)]);
    });
}

#[test]
fn an_open_failure_moves_to_provider_unavailable_and_keeps_no_text() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let client = ScriptedClient::new(Script::FailOpen);
        let mut session = text();
        let (report, events) = drive(&mut session, &client, &CancellationToken::new(), true).await;

        assert_eq!(report.outcome, ReplyOutcome::ProviderUnavailable);
        assert!(report.text.is_empty());
        assert_eq!(session.phase(), Phase::ProviderUnavailable);
        assert_eq!(events, [UiEvent::session_state(Phase::ProviderUnavailable)]);
    });
}

#[test]
fn a_stream_error_before_any_text_counts_as_empty_output() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let client = ScriptedClient::new(Script::Steps {
            steps: vec![Step::StreamError],
            after: After::Close,
        });
        // With the fallback still available the authored line is spoken.
        let mut session = voice();
        let (report, _) = drive(&mut session, &client, &CancellationToken::new(), true).await;
        assert_eq!(report.outcome, ReplyOutcome::Fallback);
        assert_eq!(report.text, tutor_engine::FALLBACK_LINE);

        // With none left, the provider is treated as failing.
        let mut session = voice();
        let (report, events) = drive(&mut session, &client, &CancellationToken::new(), false).await;
        assert_eq!(report.outcome, ReplyOutcome::ProviderUnavailable);
        assert_eq!(session.phase(), Phase::ProviderUnavailable);
        assert_eq!(events, [UiEvent::session_state(Phase::ProviderUnavailable)]);
    });
}

#[test]
fn a_stream_error_after_text_keeps_it_and_reports_truncated() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let client = ScriptedClient::new(Script::Steps {
            steps: vec![Step::Text("One sentence. And then"), Step::StreamError],
            after: After::Close,
        });
        let mut session = voice();
        let (report, events) = drive(&mut session, &client, &CancellationToken::new(), true).await;

        assert_eq!(report.outcome, ReplyOutcome::Truncated);
        assert_eq!(report.text, "One sentence. And then");
        assert_eq!(report.sentences, 2);
        assert_eq!(session.turn(), Some(TurnState::Listening));
        assert_eq!(session.turns_completed(), 1);
        assert!(events.contains(&UiEvent::TutorSentenceSpoken {
            index: 1,
            text: "And then".to_owned()
        }));
    });
}

#[test]
fn cancelling_before_the_first_token_pauses_the_session() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let client = ScriptedClient::new(Script::Steps {
            steps: vec![],
            after: After::Silent,
        });
        let mut session = voice();
        let cancel = CancellationToken::new();
        let mut events = Vec::new();
        let mut emit = |e| events.push(e);
        let report = tokio::join!(
            run_reply(&mut session, &client, request(), &cancel, true, &mut emit),
            async {
                tokio::time::sleep(Duration::from_millis(20)).await;
                cancel.cancel();
            }
        )
        .0
        .unwrap();

        assert_eq!(report.outcome, ReplyOutcome::Stopped);
        assert!(report.text.is_empty());
        assert_eq!(session.phase(), Phase::Paused);
        assert_eq!(session.turns_completed(), 0);
        assert_eq!(events, [UiEvent::session_state(Phase::Paused)]);
    });
}

#[test]
fn cancelling_after_text_keeps_it_and_returns_to_idle() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let client = ScriptedClient::new(Script::Steps {
            steps: vec![Step::Text("I started to say. ")],
            after: After::Silent,
        });
        let mut session = voice();
        let cancel = CancellationToken::new();
        let mut events = Vec::new();
        let mut emit = |e| events.push(e);
        let report = tokio::join!(
            run_reply(&mut session, &client, request(), &cancel, true, &mut emit),
            async {
                tokio::time::sleep(Duration::from_millis(20)).await;
                cancel.cancel();
            }
        )
        .0
        .unwrap();

        assert_eq!(report.outcome, ReplyOutcome::Stopped);
        assert_eq!(report.text, "I started to say. ");
        assert_eq!(session.turn(), Some(TurnState::Listening));
        assert_eq!(session.turns_completed(), 0);
        assert!(events.contains(&UiEvent::TutorSentenceSpoken {
            index: 0,
            text: "I started to say.".to_owned()
        }));
        assert!(events.contains(&UiEvent::turn_state(TurnState::Listening)));
    });
}

#[test]
fn the_request_reaches_the_client_unchanged() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let client = ScriptedClient::new(Script::Steps {
            steps: vec![Step::Text("Hello."), Step::Finished],
            after: After::Close,
        });
        let mut session = text();
        let expected = request();
        let mut events = Vec::new();
        run_reply(
            &mut session,
            &client,
            expected.clone(),
            &CancellationToken::new(),
            true,
            &mut |e| events.push(e),
        )
        .await
        .unwrap();
        assert_eq!(client.requests(), [expected]);
    });
}
