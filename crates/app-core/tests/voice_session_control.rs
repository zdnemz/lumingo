#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! What the voice loop gives a program that runs sessions for a screen: speak a
//! stored text, correct a transcript, read the microphone level, hear when a turn
//! is stored and when its analysis is done.

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use app_core::voice::testing::{ReplyScript, ScriptedLlm, Step};
use app_core::voice::{EditEffect, Recording, VoiceError, VoiceEvent};
use async_trait::async_trait;
use common::voice::{Audio, Options, Rig};
use llm_client::{
    Capabilities, LlmClient, LlmError, StructuredOutput, StructuredRequest, TextRequest, TextStream,
};
use storage::{Database, InputMode, L1HelpMode, NewProfile, Timestamp, TurnRole, UiLanguage};
use tokio_util::sync::CancellationToken;
use tutor_engine::{Phase, TurnState};

fn clock() -> tutor_engine::Clock {
    let ticks = Arc::new(Mutex::new(0_i64));
    Arc::new(move || {
        let mut n = ticks.lock().unwrap();
        *n += 1;
        Timestamp::from_unix_seconds(1_790_000_000 + *n).expect("timestamp")
    })
}

async fn recording(dir: &tempfile::TempDir) -> (Recording, Database) {
    let db = Database::open(dir.path().join("lumingo.sqlite"))
        .await
        .expect("open the database");
    let profile = db
        .profiles()
        .create(&NewProfile {
            display_name: "Test learner".to_owned(),
            ui_language: UiLanguage::Id,
            l1: "id".to_owned(),
            l1_help_mode: L1HelpMode::Auto,
            created_at: Timestamp::from_unix_seconds(1_790_000_000).expect("timestamp"),
        })
        .await
        .expect("profile");
    (
        Recording {
            db: db.clone(),
            profile_id: profile.id,
            provider_profile_id: None,
            model: "test-model".to_owned(),
            app_version: "0.0.0".to_owned(),
            link_unit: false,
            clock: clock(),
        },
        db,
    )
}

/// The scripted client, except that its analysis calls answer 429 while `limited`
/// is set. A rate limit moves the analyzer to batched cadence, so the turn stays
/// queued and a correction still reaches it.
struct Limited {
    inner: Arc<ScriptedLlm>,
    limited: AtomicBool,
    asked: Mutex<Vec<String>>,
}

#[async_trait]
impl LlmClient for Limited {
    async fn stream_text(
        &self,
        request: TextRequest,
        cancel: CancellationToken,
    ) -> Result<TextStream, LlmError> {
        self.inner.stream_text(request, cancel).await
    }

    async fn structured(
        &self,
        request: StructuredRequest,
        cancel: CancellationToken,
    ) -> Result<StructuredOutput, LlmError> {
        self.asked.lock().unwrap().push(
            request
                .messages
                .iter()
                .map(|m| m.content.clone())
                .collect::<Vec<_>>()
                .join("\n"),
        );
        if self.limited.load(Ordering::SeqCst) {
            return Err(LlmError::RateLimited { retry_after: None });
        }
        self.inner.structured(request, cancel).await
    }

    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }
}

async fn until(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..1_000 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("timed out waiting for {what}");
}

fn reply(text: &str) -> Step {
    Step::Reply(ReplyScript::new(&[text]))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stored_text_is_spoken_without_the_model_and_without_a_tutor_turn() {
    let mut rig = Rig::start(Vec::new(), Options::default()).await;
    let tts = rig.tts_log.clone().expect("a synthesiser");

    let taken = rig.handle.say("Here is the line you saved.").await.unwrap();
    assert!(taken, "an idle loop that has speech output takes the text");
    until("the text to be spoken", || {
        tts.spoken.lock().unwrap().len() == 1
    })
    .await;
    until("the loop to listen again", || {
        rig.handle.phase()
            == Phase::Active {
                turn: TurnState::Listening,
            }
    })
    .await;

    assert_eq!(
        tts.spoken.lock().unwrap().as_slice(),
        ["Here is the line you saved."]
    );
    assert_eq!(rig.llm.calls(), 0, "the model was not asked");
    let events = rig.log.all();
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, VoiceEvent::TutorSentence { .. } | VoiceEvent::Latency(_))),
        "a stored text is not a tutor sentence and has no latency: {events:#?}"
    );
    assert_eq!(rig.handle.current_turn(), 1, "it takes a turn number");
    let summary = rig.finish().await;
    assert!(
        summary.latencies.is_empty(),
        "a stored text has no latency record: {summary:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stored_text_is_refused_while_the_tutor_is_busy_and_when_there_is_no_speech_output() {
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let steps = vec![Step::Reply(
        ReplyScript::new(&["First part. ", "Second part."]).hold_after(0, release.clone()),
    )];
    let mut rig = Rig::start(steps, Options::default()).await;
    rig.handle.send_text("Hello").await.unwrap();
    until("the reply to start", || {
        rig.log
            .sentences()
            .iter()
            .any(|s| s.starts_with("First part"))
    })
    .await;
    let busy = rig.handle.say("Not now.").await.unwrap();
    assert!(!busy, "a turn is under way");
    release.add_permits(8);
    rig.log.turn_ended(1).await;
    let _ = rig.finish().await;

    let mut silent = Rig::start(
        Vec::new(),
        Options {
            audio: Audio::None,
            listen: false,
            tts: false,
            ..Options::default()
        },
    )
    .await;
    assert!(
        !silent.handle.say("No voice here.").await.unwrap(),
        "without speech output there is nothing to speak with"
    );
    let _ = silent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_correction_before_the_turn_is_stored_is_the_stored_text_and_keeps_the_original() {
    let dir = tempfile::tempdir().expect("dir");
    let (recording, db) = recording(&dir).await;
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let steps = vec![Step::Reply(
        ReplyScript::new(&["Nice to meet you. ", "How are you?"]).hold_after(0, release.clone()),
    )];
    let mut rig = Rig::start(
        steps,
        Options {
            transcripts: vec!["my name is dewy"],
            recording: Some(recording),
            ..Options::default()
        },
    )
    .await;
    rig.say().await;
    rig.log
        .wait("the first sentence of the reply", |events| {
            events
                .iter()
                .any(|e| matches!(e, VoiceEvent::TutorSentence { .. }))
        })
        .await;

    let effect = rig
        .handle
        .edit_transcript(1, "My name is Dewi.")
        .await
        .unwrap();
    assert_eq!(effect, EditEffect::BeforeRecording);
    release.add_permits(8);
    let events = rig.log.turn_ended(1).await;
    let recorded = rig
        .log
        .wait("the stored turn", |events| {
            events
                .iter()
                .any(|e| matches!(e, VoiceEvent::Recorded { .. }))
        })
        .await;
    assert!(events.len() <= recorded.len());

    let summary = rig.finish().await;
    let record = summary.recording.expect("a recording");
    let turns = db.turns().list(record.session_id).await.unwrap();
    let learner = turns
        .iter()
        .find(|t| t.role == TurnRole::Learner)
        .expect("the learner's turn");
    assert_eq!(learner.text, "My name is Dewi.");
    assert_eq!(learner.stt_text.as_deref(), Some("my name is dewy"));
    assert!(learner.edited_by_learner);
    assert_eq!(learner.input_mode, InputMode::Voice);
    assert!(
        recorded.iter().any(|e| matches!(
            e,
            VoiceEvent::Recorded { turn: 1, learner_seq, text }
                if *learner_seq == learner.seq && text == "My name is Dewi."
        )),
        "the stored event carries the stored position and the corrected text: {recorded:#?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_correction_while_the_analysis_is_running_is_stored_but_is_not_what_was_analysed() {
    let dir = tempfile::tempdir().expect("dir");
    let (recording, db) = recording(&dir).await;
    let mut rig = Rig::start(
        vec![reply("Good to hear.")],
        Options {
            transcripts: vec!["I like tea"],
            recording: Some(recording),
            ..Options::default()
        },
    )
    .await;
    rig.llm.set_structured_delay(Duration::from_millis(600));
    rig.say().await;
    rig.log
        .wait("the stored turn", |events| {
            events
                .iter()
                .any(|e| matches!(e, VoiceEvent::Recorded { .. }))
        })
        .await;
    let llm = rig.llm.clone();
    until("the analysis call to start", move || {
        llm.structured_calls() == 1
    })
    .await;

    let effect = rig
        .handle
        .edit_transcript(1, "I like green tea")
        .await
        .unwrap();
    assert_eq!(effect, EditEffect::StoredOnly);
    let events = rig
        .log
        .wait("the analysis to be reported", |events| {
            events.iter().any(|e| matches!(e, VoiceEvent::Analysed(_)))
        })
        .await;
    let analysed = events
        .iter()
        .find_map(|e| match e {
            VoiceEvent::Analysed(report) => Some(report),
            _ => None,
        })
        .expect("an analysis report");
    assert_eq!(analysed.analysed.len(), 1);

    let summary = rig.finish().await;
    let record = summary.recording.expect("a recording");
    let turns = db.turns().list(record.session_id).await.unwrap();
    let learner = turns.iter().find(|t| t.role == TurnRole::Learner).unwrap();
    assert_eq!(learner.text, "I like green tea");
    assert_eq!(learner.stt_text.as_deref(), Some("I like tea"));
    assert!(learner.edited_by_learner);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_correction_of_a_turn_that_waits_for_its_analysis_is_what_gets_analysed() {
    let dir = tempfile::tempdir().expect("dir");
    let (recording, db) = recording(&dir).await;
    let scripted = ScriptedLlm::new(vec![reply("Good to hear.")], None);
    let client = Arc::new(Limited {
        inner: scripted,
        limited: AtomicBool::new(true),
        asked: Mutex::new(Vec::new()),
    });
    let mut rig = Rig::start(
        Vec::new(),
        Options {
            transcripts: vec!["My name is Dewy."],
            recording: Some(recording),
            llm: Some(client.clone()),
            ..Options::default()
        },
    )
    .await;
    rig.say().await;
    rig.log
        .wait("the stored turn", |events| {
            events
                .iter()
                .any(|e| matches!(e, VoiceEvent::Recorded { .. }))
        })
        .await;

    // The first analysis call is answered with a 429, so the turn waits for the
    // next batch. Until that call has settled the turn counts as in flight, so the
    // correction is repeated until it finds the turn waiting.
    let mut effect = EditEffect::StoredOnly;
    for _ in 0..400 {
        effect = rig
            .handle
            .edit_transcript(1, "My name is Dewi Lestari.")
            .await
            .unwrap();
        if effect == EditEffect::AnalysedFromEdit {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(effect, EditEffect::AnalysedFromEdit);

    client.limited.store(false, Ordering::SeqCst);
    let summary = rig.finish().await;
    let record = summary.recording.expect("a recording");
    assert_eq!(
        record.unanalysed_turns, 0,
        "finishing analysed the waiting turn"
    );
    let last = client
        .asked
        .lock()
        .unwrap()
        .last()
        .cloned()
        .expect("a call");
    assert!(
        last.contains("My name is Dewi Lestari."),
        "the analysis was of the corrected words"
    );
    assert!(
        !last.contains("Dewy"),
        "the recogniser's words are not what was analysed"
    );
    let turns = db.turns().list(record.session_id).await.unwrap();
    let learner = turns.iter().find(|t| t.role == TurnRole::Learner).unwrap();
    assert!(db.analysis().get(learner.id).await.unwrap().is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn only_a_spoken_and_stored_turn_can_be_corrected() {
    let dir = tempfile::tempdir().expect("dir");
    let (recording, _db) = recording(&dir).await;
    let mut rig = Rig::start(
        vec![reply("Hello.")],
        Options {
            recording: Some(recording),
            ..Options::default()
        },
    )
    .await;
    rig.handle.send_text("I typed this").await.unwrap();
    rig.log.turn_ended(1).await;
    rig.log
        .wait("the stored turn", |events| {
            events
                .iter()
                .any(|e| matches!(e, VoiceEvent::Recorded { .. }))
        })
        .await;
    assert!(matches!(
        rig.handle.edit_transcript(1, "changed").await,
        Err(VoiceError::NotSpoken)
    ));
    assert!(matches!(
        rig.handle.edit_transcript(99, "changed").await,
        Err(VoiceError::NoSuchTurn)
    ));
    let _ = rig.finish().await;

    let mut unrecorded = Rig::start(Vec::new(), Options::default()).await;
    assert!(matches!(
        unrecorded.handle.edit_transcript(1, "changed").await,
        Err(VoiceError::NotRecording)
    ));
    let _ = unrecorded.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_stored_session_id_is_known_only_when_the_loop_stores() {
    let dir = tempfile::tempdir().expect("dir");
    let (recording, db) = recording(&dir).await;
    let mut stored = Rig::start(
        Vec::new(),
        Options {
            recording: Some(recording),
            ..Options::default()
        },
    )
    .await;
    let id = stored.handle.session_id().expect("a stored session");
    assert!(db.sessions().get(id).await.unwrap().is_some());
    let _ = stored.finish().await;

    let mut plain = Rig::start(Vec::new(), Options::default()).await;
    assert_eq!(plain.handle.session_id(), None);
    let _ = plain.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_microphone_level_follows_the_input_and_each_read_starts_a_new_window() {
    let mut rig = Rig::start(
        vec![reply("Hello.")],
        Options {
            audio: Audio::Full,
            transcripts: vec!["hello"],
            ..Options::default()
        },
    )
    .await;
    let backend = rig
        .audio
        .as_ref()
        .expect("fake audio")
        .backend
        .inner
        .clone();
    assert!(rig.handle.mic_level() < 0.01, "silent before any sound");

    backend.set_input_tone(Some(440.0));
    let handle = rig.handle.clone();
    let mut loud = 0.0_f32;
    for _ in 0..400 {
        loud = loud.max(handle.mic_level());
        if loud > 0.2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(loud > 0.2, "a tone reaches the meter: {loud}");

    backend.set_input_tone(None);
    // Frames already in flight finish, then every window is silent.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let _ = rig.handle.mic_level();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        rig.handle.mic_level() < 0.01,
        "silence after the tone stopped"
    );
    let _ = rig.finish().await;
}
