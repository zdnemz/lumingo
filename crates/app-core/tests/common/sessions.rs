#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
#![allow(dead_code)]

//! A core with the session manager attached over fakes: a scripted language
//! model, fake speech engines and a fake audio backend, and a log of every event
//! the stream carries.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use app_core::api::{
    ActiveSessionView, ServerEvent, SessionKind, StartSessionRequest, TopicChoice,
};
use app_core::engines::Engines;
use app_core::engines::testing::{FakeEngines, FakeSpeech, engines_over, fake_engines};
use app_core::voice::testing::{FakeAudio, ScriptedLlm, Step};
use app_core::{AppCore, SessionManager};
use tokio::sync::broadcast;

use super::seed::write_example_unit;
use super::{ManualClock, TestCore, config};

/// Every event of the stream, collected as it happens.
#[derive(Clone)]
pub struct Events {
    log: Arc<Mutex<Vec<ServerEvent>>>,
    core: Arc<AppCore>,
}

impl Events {
    pub fn collect(core: &Arc<AppCore>) -> Self {
        let mut rx = core.events().subscribe();
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(event) => sink.lock().unwrap().push(event),
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        Self {
            log: events,
            core: Arc::clone(core),
        }
    }

    /// The events so far, once the log has caught up with everything published.
    pub async fn settled(&self) -> Vec<ServerEvent> {
        for _ in 0..2_000 {
            let events = self.all();
            if events
                .last()
                .is_none_or(|e| e.seq() == self.core.events().current_seq())
            {
                return events;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("the event log did not catch up");
    }

    pub fn all(&self) -> Vec<ServerEvent> {
        self.log.lock().unwrap().clone()
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.all().iter().map(ServerEvent::name).collect()
    }

    /// Waits until `done` holds for the events so far.
    pub async fn wait(
        &self,
        what: &str,
        done: impl Fn(&[ServerEvent]) -> bool,
    ) -> Vec<ServerEvent> {
        for _ in 0..2_000 {
            let events = self.all();
            if done(&events) {
                return events;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!(
            "timed out waiting for {what}; events so far: {:#?}",
            self.names()
        );
    }

    /// Waits until a reply has been stored: a text session's opening line, or the
    /// end of a turn.
    pub async fn reply_stored(&self, from: usize) -> Vec<ServerEvent> {
        self.wait("a reply to be stored", |events| {
            events.iter().skip(from).any(|e| {
                matches!(
                    e,
                    ServerEvent::TurnState {
                        tutor_turn_seq: Some(_),
                        ..
                    }
                )
            })
        })
        .await
    }

    /// Waits until an event named `name` has come at or after position `from`.
    pub async fn wait_for(&self, name: &str, from: usize) -> Vec<ServerEvent> {
        self.wait(name, |events| {
            events.iter().skip(from).any(|e| e.name() == name)
        })
        .await
    }
}

/// What a rig is built with.
pub struct Setup {
    /// The example unit is in the unit folder.
    pub unit: bool,
    /// The replies of the language model, in order.
    pub steps: Vec<Step>,
    /// Fake devices and speech engines that recognise these transcripts. `None`:
    /// no audio and no speech, as in a build with every feature off.
    pub speech: Option<Vec<&'static str>>,
    /// The catalogs folder of the repository, so rubrics are there.
    pub catalogs: bool,
    /// The scripted model stands in for the active provider. Without it the core
    /// has no provider at all, as on a first run.
    pub llm: bool,
    /// The speech engines are there but the recogniser fails to load.
    pub failing_stt: bool,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            unit: true,
            steps: Vec::new(),
            speech: None,
            catalogs: true,
            llm: true,
            failing_stt: false,
        }
    }
}

pub struct SessionRig {
    pub t: TestCore,
    pub manager: Arc<SessionManager>,
    pub events: Events,
    pub llm: Arc<ScriptedLlm>,
    pub audio: Option<FakeAudio>,
    pub speech: Option<Arc<FakeSpeech>>,
}

impl SessionRig {
    pub fn core(&self) -> &Arc<AppCore> {
        &self.t.core
    }

    /// Starts a session and returns its view.
    pub async fn start(&self, request: StartSessionRequest) -> ActiveSessionView {
        self.t
            .core
            .start_session(request)
            .await
            .expect("the session starts")
    }

    /// The number of events so far, once the log has caught up, to wait for what
    /// comes after.
    pub async fn mark(&self) -> usize {
        self.events.settled().await.len()
    }
}

pub async fn rig(setup: Setup) -> SessionRig {
    let dir = tempfile::tempdir().expect("temp dir");
    if setup.unit {
        write_example_unit(&dir);
    }
    let clock = ManualClock::new("2026-10-05T08:00:00.000Z", "2026-10-05");
    let mut config = config(&dir, &clock, &[]);
    if setup.catalogs {
        config.catalogs_dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../curriculum/catalogs");
    }
    let core = AppCore::open(config).await.expect("open the core");
    let (engines, audio, speech) = match &setup.speech {
        Some(transcripts) => {
            let FakeEngines {
                engines,
                audio,
                speech,
            } = if setup.failing_stt {
                engines_over(FakeSpeech::failing_stt())
            } else {
                fake_engines(transcripts)
            };
            (engines, Some(audio), Some(speech))
        }
        None => (
            Engines::without("this test has no audio and no speech"),
            None,
            None,
        ),
    };
    let manager = SessionManager::attach(&core, engines)
        .await
        .expect("attach the session manager");
    let llm = ScriptedLlm::new(setup.steps, None);
    if setup.llm {
        manager.use_llm(llm.clone());
    }
    let events = Events::collect(&core);
    SessionRig {
        t: TestCore { core, dir, clock },
        manager,
        events,
        llm,
        audio,
        speech,
    }
}

pub fn chat_request(topic: &str) -> StartSessionRequest {
    StartSessionRequest {
        kind: SessionKind::TextChat,
        unit_id: None,
        activity_id: None,
        mode: None,
        topic: Some(TopicChoice::Typed {
            text: topic.to_owned(),
        }),
        level: None,
        speak: None,
    }
}

pub fn request(kind: SessionKind) -> StartSessionRequest {
    StartSessionRequest {
        kind,
        unit_id: None,
        activity_id: None,
        mode: None,
        topic: None,
        level: None,
        speak: None,
    }
}

pub fn unit_request(kind: SessionKind, unit_id: &str) -> StartSessionRequest {
    StartSessionRequest {
        unit_id: Some(unit_id.to_owned()),
        ..request(kind)
    }
}
