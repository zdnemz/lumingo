//! The session manager: at most one active session, and the wiring of the
//! engines to the event bus.
//!
//! # Kinds of session (`context_pack.md` section 9)
//!
//! | Kind | Runs on | Needs |
//! |---|---|---|
//! | `text_chat` | `tutor_engine::TextChat` | a provider |
//! | `conversation` | [`crate::voice::VoiceLoop`] | speech engines and audio devices (replies are only shown with `speak: false`, which still needs the microphone path) |
//! | `lesson`, `checkpoint`, `drill` | `tutor_engine::UnitPlayer` | the unit; a provider for the roleplay and the productive activities, which wait without one |
//! | `writing` | `tutor_engine::Workshop` | a provider for layers two and three; layer one and the queue work without |
//! | `reading` | `tutor_engine::ReadingSession` | a provider to generate; authored sets are the fallback |
//! | `review` | none | `NotAvailable`: the review queue has a scheduler and no presenter of its items |
//! | `placement` | none | `NotAvailable`: the placement item bank does not exist |
//!
//! # Rules
//!
//! * One session at a time. A start while another runs, or is starting, is
//!   refused with `Conflict`; nothing is queued.
//! * One turn at a time inside a session. A second turn while one runs is refused
//!   with `Busy`.
//! * The session lives here, not in the browser. A closed or reloaded page does
//!   not end it, and a page that comes back reads it from the snapshot.
//! * Every task a session starts has a child of the session's cancellation token,
//!   which is a child of the program's. Stopping the program stops them all.
//! * No lock is held across an await that waits for the provider or an engine,
//!   except the session's own chat lock, which is the point of it: a turn holds it
//!   for as long as it runs, and a stop cancels the turn before it asks for it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use async_trait::async_trait;
use llm_client::LlmClient;
use storage::SessionStatus;
use tokio_util::sync::CancellationToken;

use super::catalogs::Catalogs;
use super::emit::Emitter;
use super::env::{Env, missing_engine};
use crate::api::{
    Ack, ActiveSessionView, ActivityAnswer, AnswerReadingRequest, AudioDevices, AudioTestReport,
    AudioTestRequest, DraftAccepted, EditResult, EngineId, EngineView, Feature, FeedbackView,
    GenerateReadingRequest, NextActivity, ReadingAnswered, ReadingOutcomeView, ServerEvent,
    SessionEnded, SessionKind, SessionLife, SpeakAccepted, SpeakRequest, StartSessionRequest,
    StopRequest, SubmitActivityRequest, SubmitActivityResponse, TopicChoice, TurnAccepted,
};
use crate::core::AppCore;
use crate::engines::Engines;
use crate::error::{CoreError, CoreResult};
use crate::events::EventBus;
use crate::session::SessionService;

/// How long a normal stop waits for the work in flight (a running analysis, the
/// last turns) before it cancels what is left.
pub(crate) const STOP_WAIT: Duration = Duration::from_secs(20);

/// How long the program waits for the session to end at shutdown before it
/// stops it the hard way.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(10);

/// What a session ended with.
pub(crate) struct Ended {
    pub status: SessionStatus,
    pub feedback: Option<FeedbackView>,
}

fn unsupported<T>(what: &str) -> CoreResult<T> {
    Err(CoreError::Conflict(format!(
        "this kind of session does not take {what}"
    )))
}

/// A running session. Every kind implements the commands it takes; the rest
/// answer `Conflict` and name what they do not take.
#[async_trait]
pub(crate) trait Run: Send + Sync {
    fn emitter(&self) -> &Emitter;

    /// Called once, right after the manager published `SessionState` `active`: a
    /// session that speaks first, or has workers to start, does it here, so the
    /// first event of every session is its `SessionState`.
    async fn begin(&self) {}

    async fn pause(&self) -> CoreResult<()> {
        unsupported("a pause")
    }

    async fn resume(&self) -> CoreResult<()> {
        unsupported("a resume")
    }

    async fn send_text(&self, _text: String) -> CoreResult<TurnAccepted> {
        unsupported("typed messages")
    }

    async fn edit_turn(&self, _turn: u64, _text: String) -> CoreResult<crate::api::EditEffect> {
        unsupported("a corrected transcript")
    }

    async fn push_to_talk(&self, _pressed: bool) -> CoreResult<()> {
        unsupported("push-to-talk")
    }

    async fn stop_speaking(&self) -> CoreResult<()> {
        unsupported("a stop of the tutor's voice")
    }

    async fn next_activity(&self) -> CoreResult<NextActivity> {
        unsupported("activities")
    }

    async fn submit_activity(
        &self,
        _activity_id: String,
        _answer: ActivityAnswer,
    ) -> CoreResult<SubmitActivityResponse> {
        unsupported("answers to activities")
    }

    async fn submit_draft(&self, _text: String) -> CoreResult<DraftAccepted> {
        unsupported("writing drafts")
    }

    async fn generate_reading(&self, _topic: TopicChoice) -> CoreResult<ReadingOutcomeView> {
        unsupported("reading texts")
    }

    async fn answer_reading(&self, _request: AnswerReadingRequest) -> CoreResult<ReadingAnswered> {
        unsupported("answers to reading questions")
    }

    /// The audio lines of an activity, for the speech output. Each call counts as
    /// one play of the item.
    async fn activity_audio(&self, _activity_id: &str) -> CoreResult<String> {
        unsupported("the audio of an activity")
    }

    /// The voice loop that can speak a stored text, when the session has one with
    /// speech output.
    fn speaker(&self) -> Option<crate::voice::VoiceHandle> {
        None
    }

    /// Ends the session. `cancel` aborts it: no more analysis is started and
    /// there is no summary.
    async fn finish(&self, cancel: bool) -> CoreResult<Ended>;
}

enum Slot {
    Idle,
    /// A start is running. The slot is taken so a second start is refused while
    /// engines load.
    Starting,
    Active {
        id: i64,
        run: Arc<dyn Run>,
    },
    /// The session is being ended. Its devices are still in use, so no other
    /// session starts and no command reaches it.
    Ending,
}

/// Gives the slot back when a start fails or its future is dropped.
struct StartGuard<'a> {
    slot: &'a Mutex<Slot>,
    done: bool,
}

impl Drop for StartGuard<'_> {
    fn drop(&mut self) {
        if !self.done {
            *lock(self.slot) = Slot::Idle;
        }
    }
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every value behind these locks is replaced whole or field by field and is
    // valid after any of those steps.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the engines look like now: what the files say, and what a session or a
/// test found out since.
#[derive(Default)]
struct EngineBook {
    base: Vec<EngineView>,
    overlay: HashMap<EngineId, EngineView>,
    speech_ready: bool,
}

/// What every session shares.
pub(crate) struct Shared {
    pub bus: Arc<EventBus>,
    pub engines: Engines,
    pub catalogs: Catalogs,
    pub shutdown: CancellationToken,
    book: Mutex<EngineBook>,
    override_client: Mutex<Option<Arc<dyn LlmClient>>>,
}

impl Shared {
    /// Records what a session found out about an engine and tells the stream.
    pub(crate) fn set_engine(&self, view: EngineView) {
        lock(&self.book).overlay.insert(view.id, view.clone());
        self.bus
            .publish(|seq| ServerEvent::EngineStatus { seq, engine: view });
    }

    pub(crate) fn override_client(&self) -> Option<Arc<dyn LlmClient>> {
        lock(&self.override_client).clone()
    }

    /// Which model an engine loaded in this run, as the engine reported itself.
    pub(crate) fn engine_model(&self, id: EngineId) -> Option<speech::EngineInfo> {
        let book = lock(&self.book);
        let model = book.overlay.get(&id)?.model.as_ref()?;
        Some(speech::EngineInfo {
            id: model.id.clone(),
            version: model.version.clone(),
            model_checksum: model.model_checksum.clone(),
        })
    }
}

/// The session service of the server: see the module documentation.
pub struct SessionManager {
    core: Weak<AppCore>,
    shared: Arc<Shared>,
    slot: Mutex<Slot>,
    speaker: super::speak::StandaloneSpeaker,
    audio_test: tokio::sync::Mutex<()>,
}

impl std::fmt::Debug for SessionManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionManager").finish_non_exhaustive()
    }
}

impl SessionManager {
    /// Creates the manager, reads the catalogs and attaches it to the core.
    pub async fn attach(core: &Arc<AppCore>, engines: Engines) -> CoreResult<Arc<Self>> {
        let config = core.config.clone();
        let catalogs = tokio::task::spawn_blocking(move || Catalogs::load(&config))
            .await
            .map_err(|_| CoreError::Internal("reading the catalogs did not finish".to_owned()))?;
        let manager = Arc::new(Self {
            core: Arc::downgrade(core),
            shared: Arc::new(Shared {
                bus: Arc::clone(&core.bus),
                engines,
                catalogs,
                shutdown: core.shutdown_token(),
                book: Mutex::new(EngineBook::default()),
                override_client: Mutex::new(None),
            }),
            slot: Mutex::new(Slot::Idle),
            speaker: super::speak::StandaloneSpeaker::default(),
            audio_test: tokio::sync::Mutex::new(()),
        });
        manager.refresh_engines().await;
        core.attach_sessions(Arc::clone(&manager) as Arc<dyn SessionService>)?;
        Ok(manager)
    }

    /// Replaces the language model of every session with `client`. For tests: the
    /// real client is the active provider's.
    #[cfg(feature = "test-support")]
    pub fn use_llm(&self, client: Arc<dyn LlmClient>) {
        *lock(&self.shared.override_client) = Some(client);
    }

    /// Looks at the files again to see which engines are configured. The snapshot
    /// reads the answer from memory.
    pub async fn refresh_engines(&self) {
        let engines = self.shared.engines.clone();
        let looked = tokio::task::spawn_blocking(move || {
            let ready = engines.speech_ready();
            (engines.views(), ready)
        })
        .await;
        match looked {
            Ok((views, ready)) => {
                let mut book = lock(&self.shared.book);
                book.base = views;
                book.speech_ready = ready;
            }
            Err(_) => tracing::warn!("looking at the engines did not finish"),
        }
    }

    fn core(&self) -> CoreResult<Arc<AppCore>> {
        self.core.upgrade().ok_or(CoreError::ShuttingDown)
    }

    fn current(&self, id: i64) -> CoreResult<Arc<dyn Run>> {
        match &*lock(&self.slot) {
            Slot::Active { id: active, run } if *active == id => Ok(Arc::clone(run)),
            Slot::Active { .. } | Slot::Starting | Slot::Ending | Slot::Idle => {
                Err(CoreError::NotFound { what: "session" })
            }
        }
    }

    fn active_run(&self) -> Option<Arc<dyn Run>> {
        match &*lock(&self.slot) {
            Slot::Active { run, .. } => Some(Arc::clone(run)),
            Slot::Starting | Slot::Ending | Slot::Idle => None,
        }
    }

    async fn env(&self, core: &Arc<AppCore>, need_provider: bool) -> CoreResult<Env> {
        Env::load(core, need_provider, self.shared.override_client()).await
    }

    async fn build(
        &self,
        core: &Arc<AppCore>,
        request: &StartSessionRequest,
    ) -> CoreResult<Arc<dyn Run>> {
        let shared = &self.shared;
        match request.kind {
            SessionKind::TextChat => {
                let env = self.env(core, true).await?;
                Ok(super::text::start(shared, env, request).await? as Arc<dyn Run>)
            }
            SessionKind::Conversation => {
                // The microphone and the speakers belong to the conversation.
                self.speaker.close().await;
                let env = self.env(core, true).await?;
                Ok(super::voice::start(shared, env, request).await? as Arc<dyn Run>)
            }
            SessionKind::Lesson | SessionKind::Checkpoint | SessionKind::Drill => {
                let env = self.env(core, false).await?;
                Ok(super::unit::start(shared, env, request).await? as Arc<dyn Run>)
            }
            SessionKind::Writing => {
                let env = self.env(core, false).await?;
                Ok(super::free::start_writing(shared, env, request).await? as Arc<dyn Run>)
            }
            SessionKind::Reading => {
                let env = self.env(core, false).await?;
                Ok(super::free::start_reading(shared, env, request).await? as Arc<dyn Run>)
            }
            SessionKind::Review => Err(missing_engine(
                "review sessions are not available: the review queue has a scheduler but no engine presents its items",
            )),
            SessionKind::Placement => Err(missing_engine(
                "the placement test is not available: its item bank does not exist yet",
            )),
        }
    }

    /// Ends the running session and gives the slot back.
    async fn end(&self, id: i64, request: StopRequest) -> CoreResult<SessionEnded> {
        // Take the session out of the slot first, so a second stop for the same
        // session finds nothing and the devices are not given to a new session
        // before this one has let go of them.
        let run = {
            let mut slot = lock(&self.slot);
            match std::mem::replace(&mut *slot, Slot::Ending) {
                Slot::Active { id: active, run } if active == id => run,
                other => {
                    *slot = other;
                    return Err(CoreError::NotFound { what: "session" });
                }
            }
        };
        let kind = run.emitter().view().kind;
        let outcome = run.finish(request.cancel).await;
        // The slot is given back whether or not the end went cleanly: a session
        // that cannot be ended must not block every later one.
        *lock(&self.slot) = Slot::Idle;
        let emit = run.emitter();
        let ended = outcome?;
        if let Some(feedback) = ended.feedback.clone() {
            emit.feedback(feedback);
        }
        emit.session_state(SessionLife::Ended, None, None);
        Ok(SessionEnded {
            id,
            kind,
            status: ended.status.into(),
            feedback: ended.feedback,
        })
    }
}

#[async_trait]
impl SessionService for SessionManager {
    fn active_session(&self) -> Option<i64> {
        match &*lock(&self.slot) {
            Slot::Active { id, .. } => Some(*id),
            // A session that is starting has already stored its row. The core must
            // not delete data under it, but the id is not known yet.
            Slot::Starting | Slot::Ending | Slot::Idle => None,
        }
    }

    async fn stop_active(&self) -> CoreResult<()> {
        self.speaker.close().await;
        let Some(id) = self.active_session() else {
            return Ok(());
        };
        let normal =
            tokio::time::timeout(SHUTDOWN_WAIT, self.end(id, StopRequest { cancel: false })).await;
        match normal {
            Ok(result) => result.map(|_| ()),
            Err(_elapsed) => {
                tracing::warn!("the session did not end in time at shutdown; cancelling it");
                match self.end(id, StopRequest { cancel: true }).await {
                    Ok(_) | Err(CoreError::NotFound { .. }) => Ok(()),
                    Err(error) => Err(error),
                }
            }
        }
    }

    fn active_view(&self) -> Option<ActiveSessionView> {
        self.active_run().map(|run| run.emitter().view())
    }

    fn engines(&self) -> Vec<EngineView> {
        let book = lock(&self.shared.book);
        book.base
            .iter()
            .map(|view| book.overlay.get(&view.id).unwrap_or(view).clone())
            .collect()
    }

    fn provides(&self, feature: Feature) -> bool {
        match feature {
            Feature::Sessions | Feature::FreeModes => true,
            Feature::Activities => self
                .core
                .upgrade()
                .is_some_and(|core| !core.list_units().units.is_empty()),
            Feature::Speech => lock(&self.shared.book).speech_ready,
            // Downloads depend on the manifest, which the core knows.
            Feature::Models => false,
        }
    }

    async fn start(&self, request: StartSessionRequest) -> CoreResult<ActiveSessionView> {
        let core = self.core()?;
        {
            let mut slot = lock(&self.slot);
            if !matches!(*slot, Slot::Idle) {
                return Err(CoreError::Conflict(
                    "a session is already running; stop it before starting another".to_owned(),
                ));
            }
            *slot = Slot::Starting;
        }
        let mut guard = StartGuard {
            slot: &self.slot,
            done: false,
        };
        let run = self.build(&core, &request).await?;
        let emit = run.emitter();
        let view = emit.view();
        *lock(&self.slot) = Slot::Active {
            id: view.id,
            run: Arc::clone(&run),
        };
        guard.done = true;
        emit.session_state(SessionLife::Active, None, None);
        run.begin().await;
        Ok(view)
    }

    async fn stop(&self, id: i64, request: StopRequest) -> CoreResult<SessionEnded> {
        self.end(id, request).await
    }

    async fn pause(&self, id: i64) -> CoreResult<ActiveSessionView> {
        let run = self.current(id)?;
        run.pause().await?;
        Ok(run.emitter().view())
    }

    async fn resume(&self, id: i64) -> CoreResult<ActiveSessionView> {
        let run = self.current(id)?;
        run.resume().await?;
        Ok(run.emitter().view())
    }

    async fn send_text(&self, id: i64, text: String) -> CoreResult<TurnAccepted> {
        self.current(id)?.send_text(text).await
    }

    async fn edit_turn(&self, id: i64, turn: u64, text: String) -> CoreResult<EditResult> {
        let run = self.current(id)?;
        let effect = run.edit_turn(turn, text).await?;
        Ok(EditResult {
            session_id: id,
            turn,
            effect,
        })
    }

    async fn push_to_talk(&self, id: i64, pressed: bool) -> CoreResult<Ack> {
        self.current(id)?.push_to_talk(pressed).await?;
        Ok(Ack { ok: true })
    }

    async fn stop_speaking(&self) -> CoreResult<Ack> {
        // A conversation stops its own tutor, whether it is thinking or speaking.
        if let Some(run) = self.active_run()
            && run.emitter().view().kind == SessionKind::Conversation
        {
            run.stop_speaking().await?;
            return Ok(Ack { ok: true });
        }
        self.speaker.stop().await?;
        Ok(Ack { ok: true })
    }

    async fn next_activity(&self, id: i64) -> CoreResult<NextActivity> {
        self.current(id)?.next_activity().await
    }

    async fn submit_activity(
        &self,
        request: SubmitActivityRequest,
    ) -> CoreResult<SubmitActivityResponse> {
        self.current(request.session_id)?
            .submit_activity(request.activity_id, request.answer)
            .await
    }

    async fn submit_draft(&self, id: i64, text: String) -> CoreResult<DraftAccepted> {
        self.current(id)?.submit_draft(text).await
    }

    async fn generate_reading(
        &self,
        request: GenerateReadingRequest,
    ) -> CoreResult<ReadingOutcomeView> {
        self.current(request.session_id)?
            .generate_reading(request.topic)
            .await
    }

    async fn answer_reading(
        &self,
        id: i64,
        request: AnswerReadingRequest,
    ) -> CoreResult<ReadingAnswered> {
        self.current(id)?.answer_reading(request).await
    }

    async fn speak(&self, request: SpeakRequest) -> CoreResult<SpeakAccepted> {
        let core = self.core()?;
        // Looked up before anything is spoken or counted: an unknown session is
        // refused here.
        let activity_run = match &request {
            SpeakRequest::Activity { session_id, .. } => Some(self.current(*session_id)?),
            SpeakRequest::Turn { .. } | SpeakRequest::Reading { .. } => None,
        };
        // A stored turn or text is read now, so a reference to nothing is a 404
        // whatever the speech output is doing. Only the audio of an activity waits,
        // because reading it counts a play.
        let stored = match &request {
            SpeakRequest::Activity { .. } => None,
            other => Some(super::speak::stored_text(&core, other).await?),
        };
        let route = match self.active_run() {
            Some(run) if run.emitter().view().kind == SessionKind::Conversation => {
                match run.speaker() {
                    Some(handle) => super::speak::SpeakRoute::Through(handle),
                    None => super::speak::SpeakRoute::Blocked,
                }
            }
            _ => super::speak::SpeakRoute::Standalone,
        };
        let fetch = async {
            match (&request, activity_run, stored) {
                (SpeakRequest::Activity { activity_id, .. }, Some(run), _) => {
                    run.activity_audio(activity_id).await
                }
                (_, _, Some(text)) => Ok(text),
                _ => Err(CoreError::NotFound { what: "activity" }),
            }
        };
        super::speak::speak(&self.shared, &self.speaker, route, fetch).await
    }

    async fn audio_devices(&self) -> CoreResult<AudioDevices> {
        super::speak::audio_devices(&self.shared).await
    }

    async fn audio_test(&self, request: AudioTestRequest) -> CoreResult<AudioTestReport> {
        let _one = self.audio_test.try_lock().map_err(|_| CoreError::Busy)?;
        if self
            .active_run()
            .is_some_and(|run| run.emitter().view().kind == SessionKind::Conversation)
        {
            return Err(CoreError::Conflict(
                "a voice conversation is using the audio devices; stop it first".to_owned(),
            ));
        }
        self.speaker.close().await;
        super::speak::audio_test(&self.shared, request).await
    }
}
