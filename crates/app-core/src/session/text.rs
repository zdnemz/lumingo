//! Text turns: the text chat, and the roleplay inside a lesson.
//!
//! [`TextDriver`] runs one turn at a time as a task of its own, so the HTTP
//! request that started it returns at once and a closed tab does not stop it. It
//! turns what the engine does into events in a fixed order:
//!
//! 1. `TranscriptFinal` with the learner's line and its stored position, then
//!    `TurnState` `thinking`, when the engine has stored the message (the chat
//!    observer says so). The tutor's opening turn has no learner line and sends
//!    `TurnState` `thinking` first.
//! 2. `TurnState` `replying` with the first words, and one `TutorTextDelta` per
//!    piece.
//! 3. `TurnState` `waiting` with the stored position of the reply.
//! 4. `AnalysisReady` for the learner's line, held until step 3 so it can never
//!    arrive before the reply it belongs to ([`Gate`]).
//!
//! A reply that cannot be had ends in `SessionState` `provider_unavailable`; a
//! stop ends in `SessionState` `paused`.

use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use storage::{Database, SessionStatus};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tutor_engine::{
    ChatConfig, ChatDeps, ChatObserver, ChatReply, ChatTopic, EndReason, EngineError,
    FeedbackMode as EngineMode, Phase, ReplyOutcome, RunReport, TextChat, TurnState,
    session_summary,
};

use super::convert::{self, conversation_summary};
use super::emit::Emitter;
use super::env::{Env, assessment_level, engine_error};
use super::manager::{Ended, Run, STOP_WAIT, Shared, lock};
use crate::api::{
    ActiveSessionView, FeedbackView, SessionChannel, SessionKind, SessionLife, StartSessionRequest,
    TopicChoice, TurnAccepted, TurnAnalysisView, TurnPhase,
};
use crate::error::{CoreError, CoreResult};

/// The longest typed message, in characters. The engine cuts nothing, so the
/// limit is the learner's: a wall of pasted text is a mistake.
pub(crate) const MAX_MESSAGE_CHARS: usize = 2_000;

const PROVIDER_MESSAGE: &str = "The language model provider could not be reached or kept failing. \
                                Check the connection and the provider settings, then resume.";

/// Holds the analysis of a turn back until its reply is complete.
pub(crate) struct Gate {
    emit: Emitter,
    state: Mutex<GateState>,
}

#[derive(Default)]
struct GateState {
    /// The stored position of the learner message whose turn is running.
    open_seq: Option<i64>,
    held: Vec<TurnAnalysisView>,
}

impl Gate {
    fn new(emit: Emitter) -> Self {
        Self {
            emit,
            state: Mutex::new(GateState::default()),
        }
    }

    fn open(&self, seq: i64) {
        lock(&self.state).open_seq = Some(seq);
    }

    /// An analysis finished. It is sent now, unless its turn is still running.
    pub(crate) fn analysed(&self, views: Vec<TurnAnalysisView>) {
        let mut now = Vec::new();
        {
            let mut state = lock(&self.state);
            for view in views {
                if state.open_seq == Some(view.turn_seq) {
                    state.held.push(view);
                } else {
                    now.push(view);
                }
            }
        }
        for view in now {
            self.emit.analysis(view);
        }
    }

    /// The turn is over: sends what was held.
    fn release(&self) {
        let held = {
            let mut state = lock(&self.state);
            state.open_seq = None;
            std::mem::take(&mut state.held)
        };
        for view in held {
            self.emit.analysis(view);
        }
    }
}

type Pending = Arc<Mutex<Option<(u64, String)>>>;

/// Tells the stream what the engine does outside the call that is running.
struct Observer {
    emit: Emitter,
    gate: Arc<Gate>,
    pending: Pending,
}

impl ChatObserver for Observer {
    fn learner_turn_stored(&self, _turn_id: i64, seq: i64) {
        let taken = lock(&self.pending).take();
        if let Some((turn, text)) = taken {
            self.emit
                .learner_line(turn, &text, SessionChannel::Text, Some(seq), false);
            self.emit.turn_state(turn, TurnPhase::Thinking, None);
            self.gate.open(seq);
        }
    }

    fn analysis_finished(&self, report: &RunReport) {
        let views = report.analysed.iter().map(convert::analysis).collect();
        self.gate.analysed(views);
    }
}

/// Where the engine writes the pieces of a reply as they arrive.
pub(crate) type OnDelta = Box<dyn FnMut(&str) + Send>;

struct Running {
    token: CancellationToken,
    handle: JoinHandle<()>,
}

/// Runs the turns of a text conversation, one at a time.
pub(crate) struct TextDriver {
    emit: Emitter,
    db: Database,
    gate: Arc<Gate>,
    pending: Pending,
    busy: Arc<AtomicBool>,
    next_turn: AtomicU64,
    running: Mutex<Option<Running>>,
    session_token: CancellationToken,
}

impl TextDriver {
    pub(crate) fn new(emit: Emitter, db: Database, session_token: CancellationToken) -> Self {
        Self {
            gate: Arc::new(Gate::new(emit.clone())),
            emit,
            db,
            pending: Arc::new(Mutex::new(None)),
            busy: Arc::new(AtomicBool::new(false)),
            next_turn: AtomicU64::new(0),
            running: Mutex::new(None),
            session_token,
        }
    }

    /// What the engine reports to.
    pub(crate) fn observer(&self) -> Arc<dyn ChatObserver> {
        Arc::new(Observer {
            emit: self.emit.clone(),
            gate: Arc::clone(&self.gate),
            pending: Arc::clone(&self.pending),
        })
    }

    pub(crate) fn is_busy(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
    }

    /// Starts a turn and returns at once. `learner_text` is the message of a
    /// learner turn, `None` for the tutor's opening. `work` does the engine call
    /// with the writer of the reply and the turn's cancellation token.
    pub(crate) fn begin<F, Fut>(
        &self,
        learner_text: Option<String>,
        work: F,
    ) -> CoreResult<TurnAccepted>
    where
        F: FnOnce(OnDelta, CancellationToken) -> Fut + Send + 'static,
        Fut: Future<Output = Result<ChatReply, EngineError>> + Send + 'static,
    {
        if self.busy.swap(true, Ordering::SeqCst) {
            return Err(CoreError::Busy);
        }
        let turn = self.next_turn.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some(text) = &learner_text {
            *lock(&self.pending) = Some((turn, text.clone()));
        }
        let token = self.session_token.child_token();
        let emit = self.emit.clone();
        let db = self.db.clone();
        let gate = Arc::clone(&self.gate);
        let busy = Arc::clone(&self.busy);
        let pending = Arc::clone(&self.pending);
        let opening = learner_text.is_none();
        let turn_token = token.clone();
        let handle = tokio::spawn(async move {
            if opening {
                emit.turn_state(turn, TurnPhase::Thinking, None);
            }
            let mut replying = false;
            let writer = emit.clone();
            let on_delta: OnDelta = Box::new(move |delta| {
                if !replying {
                    replying = true;
                    writer.turn_state(turn, TurnPhase::Replying, None);
                }
                writer.text_delta(turn, delta);
            });
            let result = work(on_delta, turn_token).await;
            finish_turn(&emit, &db, turn, result).await;
            // A message that was refused before it was stored leaves its text behind.
            lock(&pending).take();
            gate.release();
            busy.store(false, Ordering::SeqCst);
        });
        *lock(&self.running) = Some(Running { token, handle });
        Ok(TurnAccepted {
            session_id: self.emit.id(),
            turn,
        })
    }

    /// Cancels the turn that runs, if any, and waits for it to end.
    pub(crate) async fn cancel_running(&self) {
        let running = lock(&self.running).take();
        if let Some(running) = running {
            running.token.cancel();
            if tokio::time::timeout(Duration::from_secs(5), running.handle)
                .await
                .is_err()
            {
                tracing::warn!("a turn did not end after it was cancelled");
            }
        }
    }
}

async fn tutor_seq(db: &Database, turn_id: Option<i64>) -> Option<i64> {
    let id = turn_id?;
    match db.turns().get(id).await {
        Ok(turn) => turn.map(|t| t.seq),
        Err(error) => {
            tracing::warn!(%error, "a stored turn could not be read");
            None
        }
    }
}

async fn finish_turn(
    emit: &Emitter,
    db: &Database,
    turn: u64,
    result: Result<ChatReply, EngineError>,
) {
    match result {
        Ok(reply) => match reply.outcome {
            ReplyOutcome::Normal | ReplyOutcome::Fallback => {
                let seq = tutor_seq(db, reply.tutor_turn_id).await;
                emit.tutor_line(turn, &reply.text, seq);
                emit.turn_state(turn, TurnPhase::Waiting, seq);
            }
            ReplyOutcome::ProviderUnavailable => {
                emit.clear_pending_reply();
                emit.session_state(
                    SessionLife::ProviderUnavailable,
                    None,
                    Some(PROVIDER_MESSAGE.to_owned()),
                );
            }
            ReplyOutcome::Stopped => {
                let seq = tutor_seq(db, reply.tutor_turn_id).await;
                emit.tutor_line(turn, &reply.text, seq);
                emit.session_state(SessionLife::Paused, None, None);
            }
        },
        Err(error) => {
            let error = engine_error(error);
            emit.clear_pending_reply();
            let body = error.body();
            emit.error(body.error, &body.message);
            emit.turn_state(turn, TurnPhase::Waiting, None);
        }
    }
}

/// After a failure that is not a refused transition, brings the chat back to
/// waiting for the learner, so one failed turn does not leave it stuck.
pub(crate) fn recover(
    chat: &mut TextChat,
    result: Result<ChatReply, EngineError>,
) -> Result<ChatReply, EngineError> {
    let stuck = matches!(
        chat.phase(),
        Phase::Active {
            turn: TurnState::Thinking | TurnState::Replying
        }
    );
    if result.is_err() && stuck {
        let _ = chat.pause();
        let _ = chat.resume();
    }
    result
}

/// [`recover`] for the roleplay of a lesson.
pub(crate) fn recover_roleplay(
    run: &mut tutor_engine::RoleplayRun,
    result: Result<ChatReply, EngineError>,
) -> Result<ChatReply, EngineError> {
    let stuck = matches!(
        run.phase(),
        Phase::Active {
            turn: TurnState::Thinking | TurnState::Replying
        }
    );
    if result.is_err() && stuck {
        let _ = run.pause();
        let _ = run.resume();
    }
    result
}

/// Checks a typed message and returns it trimmed.
pub(crate) fn checked_message(text: &str) -> CoreResult<String> {
    let text = text.trim();
    if text.is_empty() {
        return Err(CoreError::InvalidInput("the message is empty".to_owned()));
    }
    if text.chars().count() > MAX_MESSAGE_CHARS {
        return Err(CoreError::InvalidInput(format!(
            "the message is longer than {MAX_MESSAGE_CHARS} characters"
        )));
    }
    Ok(text.to_owned())
}

/// The topic of a conversation, from the request.
pub(crate) fn chat_topic(shared: &Shared, request: &StartSessionRequest) -> CoreResult<ChatTopic> {
    let level = request
        .level
        .map_or(assessment_engine::Level::A1, assessment_level);
    match &request.topic {
        None => Err(CoreError::InvalidInput(
            "a conversation needs a topic".to_owned(),
        )),
        Some(TopicChoice::Typed { text }) => {
            if tutor_engine::clean_topic(text).is_empty() {
                return Err(CoreError::InvalidInput("the topic is empty".to_owned()));
            }
            Ok(ChatTopic::Typed(text.clone()))
        }
        Some(TopicChoice::Bank { id }) => {
            let bank = shared.catalogs.topics.as_ref().ok_or_else(|| {
                CoreError::unavailable(
                    None,
                    "the topic bank is not installed (catalogs/topics.json), so only typed topics work",
                )
            })?;
            bank.conversation(level, id)
                .cloned()
                .map(ChatTopic::Bank)
                .ok_or(CoreError::NotFound { what: "topic" })
        }
    }
}

pub(crate) fn feedback_mode(request: &StartSessionRequest) -> EngineMode {
    match request.mode {
        Some(crate::api::FeedbackMode::Accuracy) => EngineMode::Accuracy,
        Some(crate::api::FeedbackMode::Fluency) | None => EngineMode::Fluency,
    }
}

/// A text chat.
pub(crate) struct TextRun {
    emit: Emitter,
    chat: Arc<tokio::sync::Mutex<TextChat>>,
    driver: TextDriver,
    session_token: CancellationToken,
}

pub(crate) async fn start(
    shared: &Arc<Shared>,
    env: Env,
    request: &StartSessionRequest,
) -> CoreResult<Arc<TextRun>> {
    let topic = chat_topic(shared, request)?;
    let level = request
        .level
        .map_or(assessment_engine::Level::A1, assessment_level);
    let chat = TextChat::start(
        ChatDeps {
            client: env.llm.clone(),
            db: env.db.clone(),
            clock: env.clock.clone(),
        },
        ChatConfig {
            profile_id: env.profile_id,
            provider_profile_id: env.provider_profile_id,
            model: env.model.clone(),
            level,
            first_language: env.first_language.clone(),
            mode: feedback_mode(request),
            topic,
            app_version: env.app_version.clone(),
        },
    )
    .await
    .map_err(engine_error)?;
    let session_token = shared.shutdown.child_token();
    let emit = Emitter::new(
        Arc::clone(&shared.bus),
        ActiveSessionView {
            id: chat.session_id(),
            kind: SessionKind::TextChat,
            unit_id: None,
            channel: SessionChannel::Text,
            life: SessionLife::Active,
            turn_state: Some(TurnPhase::Waiting),
            fault: None,
            turns_completed: 0,
            turn: 0,
            recent: Vec::new(),
            pending_reply: None,
            speaking: false,
            activity_id: None,
        },
    );
    let driver = TextDriver::new(emit.clone(), env.db.clone(), session_token.clone());
    let chat = chat.with_observer(driver.observer());
    let run = Arc::new(TextRun {
        emit,
        chat: Arc::new(tokio::sync::Mutex::new(chat)),
        driver,
        session_token,
    });
    Ok(run)
}

impl TextRun {
    /// The tutor speaks first.
    fn open_turn(&self) -> CoreResult<TurnAccepted> {
        let chat = Arc::clone(&self.chat);
        self.driver.begin(None, move |on_delta, token| async move {
            let mut chat = chat.lock().await;
            let result = chat.open(on_delta, &token).await;
            recover(&mut chat, result)
        })
    }
}

#[async_trait]
impl Run for TextRun {
    fn emitter(&self) -> &Emitter {
        &self.emit
    }

    async fn begin(&self) {
        if let Err(error) = self.open_turn() {
            tracing::warn!(%error, "the opening turn could not be started");
        }
    }

    async fn send_text(&self, text: String) -> CoreResult<TurnAccepted> {
        let text = checked_message(&text)?;
        let life = self.emit.view().life;
        if life != SessionLife::Active {
            return Err(CoreError::Conflict(
                "the session is not running: resume it first".to_owned(),
            ));
        }
        let chat = Arc::clone(&self.chat);
        let message = text.clone();
        self.driver
            .begin(Some(text), move |on_delta, token| async move {
                let mut chat = chat.lock().await;
                let result = chat.send(&message, on_delta, &token).await;
                recover(&mut chat, result)
            })
    }

    async fn pause(&self) -> CoreResult<()> {
        // A running reply is cancelled; the engine pauses itself when it stops.
        self.driver.cancel_running().await;
        let mut chat = self.chat.lock().await;
        if chat.phase() != Phase::Paused {
            chat.pause().map_err(engine_error)?;
            self.emit.session_state(SessionLife::Paused, None, None);
        }
        Ok(())
    }

    async fn resume(&self) -> CoreResult<()> {
        let mut chat = self.chat.lock().await;
        chat.resume().map_err(engine_error)?;
        let turn = self.emit.view().turn;
        self.emit.session_state(SessionLife::Active, None, None);
        self.emit.turn_state(turn, TurnPhase::Waiting, None);
        Ok(())
    }

    async fn finish(&self, cancel: bool) -> CoreResult<Ended> {
        self.driver.cancel_running().await;
        let token = self.session_token.child_token();
        let reason = if cancel {
            EndReason::Cancelled
        } else {
            EndReason::Finished
        };
        // A normal end waits for the analysis in flight, but not for ever.
        let summary = {
            let mut chat = self.chat.lock().await;
            if cancel {
                chat.finish(reason, &token).await
            } else {
                match tokio::time::timeout(STOP_WAIT, chat.finish(reason, &token)).await {
                    Ok(done) => done,
                    Err(_elapsed) => {
                        token.cancel();
                        chat.finish(EndReason::Cancelled, &self.session_token.child_token())
                            .await
                    }
                }
            }
        }
        .map_err(engine_error)?;
        let feedback = (!cancel).then(|| FeedbackView::Conversation {
            summary: conversation_summary(&summary),
        });
        Ok(Ended {
            status: if cancel {
                SessionStatus::Aborted
            } else {
                SessionStatus::Completed
            },
            feedback,
        })
    }
}

/// The summary of a conversation session read from storage, for a voice
/// session that stores its turns through the recorder.
pub(crate) async fn stored_summary(
    db: &Database,
    session_id: i64,
) -> CoreResult<crate::api::ConversationSummaryView> {
    let summary = session_summary(db, session_id, false)
        .await
        .map_err(engine_error)?;
    Ok(conversation_summary(&summary))
}
