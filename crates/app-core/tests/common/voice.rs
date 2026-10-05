#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
#![allow(dead_code)]

//! A rig for the voice loop tests: the loop over fakes, and a log of its events.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use app_core::voice::testing::{
    EnergyVad, FakeAudio, FakeStt, FakeTts, ManualClock, ScriptedLlm, Step, TtsLog, fake_audio,
};
use app_core::voice::{
    AudioMode, ListenParts, Scenario, VoiceConfig, VoiceEvent, VoiceHandle, VoiceLoop, VoiceParts,
    VoiceSummary,
};
use tokio::sync::broadcast;
use tutor_engine::Phase;

/// Every event of a run, collected as it happens.
#[derive(Clone)]
pub struct Log(Arc<Mutex<Vec<VoiceEvent>>>);

impl Log {
    pub fn collect(mut rx: broadcast::Receiver<VoiceEvent>) -> Self {
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
        Self(events)
    }

    pub fn all(&self) -> Vec<VoiceEvent> {
        self.0.lock().unwrap().clone()
    }

    /// Waits until `done` holds for the events so far.
    pub async fn wait(&self, what: &str, done: impl Fn(&[VoiceEvent]) -> bool) -> Vec<VoiceEvent> {
        for _ in 0..2_000 {
            let events = self.all();
            if done(&events) {
                return events;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!(
            "timed out waiting for {what}; events so far: {:#?}",
            self.all()
        );
    }

    /// Waits until turn `turn` ended.
    pub async fn turn_ended(&self, turn: u64) -> Vec<VoiceEvent> {
        self.wait(&format!("turn {turn} to end"), |events| {
            events
                .iter()
                .any(|e| matches!(e, VoiceEvent::TurnEnded { turn: t, .. } if *t == turn))
        })
        .await
    }

    pub fn phases(&self) -> Vec<Phase> {
        phases(&self.all())
    }

    pub fn sentences(&self) -> Vec<String> {
        self.all()
            .into_iter()
            .filter_map(|e| match e {
                VoiceEvent::TutorSentence { text, .. } => Some(text),
                _ => None,
            })
            .collect()
    }
}

pub fn phases(events: &[VoiceEvent]) -> Vec<Phase> {
    events
        .iter()
        .filter_map(|e| match e {
            VoiceEvent::State(phase) => Some(*phase),
            _ => None,
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Audio {
    None,
    /// Output only; speech is fed with `feed_audio`.
    Playback,
    /// Microphone and speakers through the gate.
    Full,
}

pub struct Options {
    pub audio: Audio,
    pub listen: bool,
    pub tts: bool,
    pub transcripts: Vec<&'static str>,
    pub provider_timeout: Duration,
    pub tts_latency: Duration,
    pub tts_slow: Duration,
    pub stt_finalise: Duration,
    pub tts_fail_on: Option<&'static str>,
    pub stt_fails: bool,
    /// A client to use instead of the scripted one.
    pub llm: Option<Arc<dyn llm_client::LlmClient>>,
    pub recording: Option<app_core::voice::Recording>,
    pub config: Box<dyn Fn(&mut VoiceConfig)>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            audio: Audio::Playback,
            listen: true,
            tts: true,
            transcripts: Vec::new(),
            provider_timeout: Duration::from_secs(16),
            tts_latency: Duration::ZERO,
            tts_slow: Duration::ZERO,
            stt_finalise: Duration::ZERO,
            tts_fail_on: None,
            stt_fails: false,
            llm: None,
            recording: None,
            config: Box::new(|_| {}),
        }
    }
}

pub struct Rig {
    pub voice: Option<VoiceLoop>,
    pub handle: VoiceHandle,
    pub log: Log,
    pub llm: Arc<ScriptedLlm>,
    pub clock: Arc<ManualClock>,
    pub tts_log: Option<Arc<TtsLog>>,
    pub audio: Option<FakeAudio>,
}

pub fn scenario() -> Scenario {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../curriculum/examples/a1-u01.example.json"
    );
    let bytes = std::fs::read(path).expect("the example unit");
    let unit = curriculum::load_unit_bytes(&bytes).expect("loads").unit;
    Scenario::from_unit(&unit, None, None, "Indonesian").expect("scenario")
}

impl Rig {
    pub async fn start(steps: Vec<Step>, options: Options) -> Self {
        Self::try_start(steps, options)
            .await
            .expect("the loop starts")
    }

    pub async fn try_start(
        steps: Vec<Step>,
        options: Options,
    ) -> Result<Self, app_core::voice::VoiceError> {
        let clock = ManualClock::new();
        let llm = ScriptedLlm::new(steps, Some(clock.clone()));
        let mut config = VoiceConfig {
            provider_timeout: options.provider_timeout,
            ..VoiceConfig::default()
        };
        (options.config)(&mut config);
        let end_silence = config.endpoint.end_silence_ms;

        let audio = (options.audio != Audio::None).then(fake_audio);
        let mode = match (&audio, options.audio) {
            (Some(a), Audio::Playback) => AudioMode::PlaybackOnly(a.registry.clone()),
            (Some(a), Audio::Full) => AudioMode::Full(a.registry.clone()),
            _ => AudioMode::None,
        };
        let listen = options.listen.then(|| ListenParts {
            vad: Box::new(EnergyVad::new(Some(clock.clone()), end_silence)),
            stt: {
                let mut stt = FakeStt::new(
                    &options.transcripts,
                    Some(clock.clone()),
                    options.stt_finalise,
                );
                if options.stt_fails {
                    stt = stt.failing();
                }
                Box::new(move || Ok(Box::new(stt) as _))
            },
        });
        let mut tts_log = None;
        let tts: Option<app_core::voice::TtsLoad> = if options.tts {
            let (mut fake, log) = FakeTts::new(Some(clock.clone()), options.tts_latency);
            if let Some(bad) = options.tts_fail_on {
                fake = fake.failing_on(bad);
            }
            if !options.tts_slow.is_zero() {
                fake = fake.slow(options.tts_slow);
            }
            tts_log = Some(log);
            Some(Box::new(move || Ok(Box::new(fake) as _)))
        } else {
            None
        };
        let voice = VoiceLoop::start(VoiceParts {
            llm: options.llm.clone().unwrap_or_else(|| llm.clone()),
            clock: clock.clone(),
            scenario: scenario(),
            config,
            audio: mode,
            listen,
            tts,
            recording: options.recording,
        })
        .await?;
        let handle = voice.handle();
        let log = Log::collect(handle.subscribe());
        Ok(Self {
            voice: Some(voice),
            handle,
            log,
            llm,
            clock,
            tts_log,
            audio,
        })
    }

    /// One utterance: 640 ms of tone, then a second of silence.
    pub async fn say(&self) {
        use app_core::voice::testing::{silence_audio, speech_audio};
        self.handle
            .feed_audio(&speech_audio(640))
            .await
            .expect("feed speech");
        self.handle
            .feed_audio(&silence_audio(1_000))
            .await
            .expect("feed silence");
    }

    pub async fn finish(&mut self) -> VoiceSummary {
        self.voice
            .take()
            .expect("not finished yet")
            .finish(false)
            .await
    }
}
