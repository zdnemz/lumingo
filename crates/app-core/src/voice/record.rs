//! Storing the turns of a voice session and running the background analysis.
//!
//! Nothing here is on the speech path. The orchestrator drops a finished turn
//! into a bounded queue (`VoiceConfig::recorder_queue`, 16) and goes on
//! listening. When the queue is full the turn is not recorded and
//! `VoiceStats::recorder_dropped` counts it: a late database never delays the
//! next reply.
//!
//! The recorder task stores each turn and hands the learner's message to the
//! tutor engine's `TurnAnalyzer`. The analysis is spawned, not awaited, so one
//! slow structured call cannot hold up the next turn's storage. The analyzer's
//! notes reach the next tutor turn through [`Recorder::notes`].

use std::collections::VecDeque;
use std::sync::Arc;

use llm_client::LlmClient;
use storage::{Database, InputMode, NewSession, NewTurn, SessionMode, SessionStatus, TurnRole};
use tokio::sync::{mpsc, oneshot};
use tokio::task::{JoinHandle, JoinSet};
use tokio_util::sync::CancellationToken;
use tutor_engine::{
    AnalysisKind, AnalyzerConfig, Clock, FeedbackMode, InputMode as AnalysisInput, TurnAnalyzer,
    TurnToAnalyse,
};

use super::config::Scenario;
use super::error::{VoiceError, VoiceResult};
use super::event::{EventSink, VoiceEvent};
use super::msg::{Counters, EditEffect};

/// Turns the recorder remembers the stored position of, so a correction can find
/// them. A correction of an older turn is refused as unknown.
const REMEMBERED_TURNS: usize = 64;

/// Where a voice session is stored. The loop does not open the database; the
/// program that runs it does, as `AppCore` does for the server.
#[derive(Clone)]
pub struct Recording {
    pub db: Database,
    pub profile_id: i64,
    pub provider_profile_id: Option<i64>,
    /// The model name stored with every analysis.
    pub model: String,
    pub app_version: String,
    /// True when the scenario's unit is in the storage index (`AppCore` writes
    /// it there at start-up). A session row refers to its unit, so a unit that
    /// is not indexed must not be named: the session is then stored without it.
    pub link_unit: bool,
    pub clock: Clock,
}

#[derive(Debug)]
pub(crate) enum RecordMsg {
    /// The tutor spoke first.
    Opening { tutor_text: String },
    Turn {
        /// The number the events of the turn carry.
        turn: u64,
        learner_text: String,
        /// The recogniser's words, when the learner corrected them before the
        /// turn was stored. `learner_text` is then the correction.
        original: Option<String>,
        input: AnalysisInput,
        speech_ms: Option<i64>,
        tutor_reply: String,
    },
    /// The learner corrected the transcript of an earlier, stored turn.
    Edit {
        turn: u64,
        text: String,
        reply: oneshot::Sender<Result<EditEffect, VoiceError>>,
    },
}

/// What a finished recording holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordSummary {
    pub session_id: i64,
    pub turns_stored: usize,
    /// Learner turns that have no analysis because a call failed or never ran.
    pub unanalysed_turns: usize,
}

pub(crate) struct Recorder {
    tx: mpsc::Sender<RecordMsg>,
    task: JoinHandle<RecordSummary>,
    analyzer: TurnAnalyzer,
    session_id: i64,
    counters: Arc<Counters>,
}

fn session_mode(mode: FeedbackMode) -> SessionMode {
    match mode {
        FeedbackMode::Fluency => SessionMode::Fluency,
        FeedbackMode::Accuracy => SessionMode::Accuracy,
    }
}

fn words(text: &str) -> i64 {
    i64::try_from(text.split_whitespace().count()).unwrap_or(i64::MAX)
}

impl Recorder {
    pub(crate) async fn start(
        recording: Recording,
        scenario: &Scenario,
        client: Arc<dyn LlmClient>,
        queue: usize,
        counters: Arc<Counters>,
        events: EventSink,
    ) -> VoiceResult<Self> {
        let stored = recording
            .db
            .sessions()
            .create(&NewSession {
                profile_id: recording.profile_id,
                kind: storage::SessionKind::Conversation,
                unit_id: scenario.unit_id.clone().filter(|_| recording.link_unit),
                activity_id: scenario.activity_id.clone().filter(|_| recording.link_unit),
                mode: Some(session_mode(scenario.context.mode)),
                provider_profile_id: recording.provider_profile_id,
                app_version: recording.app_version.clone(),
                started_at: (recording.clock)(),
            })
            .await?;
        let analyzer = TurnAnalyzer::new(
            AnalyzerConfig {
                profile_id: recording.profile_id,
                session_id: stored.id,
                provider_profile_id: recording.provider_profile_id,
                model: recording.model.clone(),
                level: scenario.context.level,
                first_language: scenario.context.first_language.clone(),
                kind: AnalysisKind::Turn,
                objectives: scenario.objectives.clone(),
                target_language: scenario.target_language.clone(),
            },
            client,
            recording.db.clone(),
            recording.clock.clone(),
        );
        let (tx, rx) = mpsc::channel(queue);
        let task = tokio::spawn(record_loop(
            rx,
            recording,
            analyzer.clone(),
            stored.id,
            events,
        ));
        Ok(Self {
            tx,
            task,
            analyzer,
            session_id: stored.id,
            counters,
        })
    }

    pub(crate) fn session_id(&self) -> i64 {
        self.session_id
    }

    /// The latest notes from the analysis, for the next tutor turn.
    pub(crate) fn notes(&self) -> Vec<String> {
        self.analyzer.notes()
    }

    pub(crate) fn push(&self, message: RecordMsg) {
        if self.tx.try_send(message).is_err() {
            Counters::bump(&self.counters.recorder_dropped);
            tracing::warn!("a turn was not recorded because the recorder was behind");
        }
    }

    /// Stores what is queued, waits for the running analyses, analyses what is
    /// waiting (unless the session was cancelled) and closes the stored session.
    pub(crate) async fn finish(self, cancelled: bool, cancel: &CancellationToken) -> RecordSummary {
        let Self {
            tx,
            task,
            analyzer,
            session_id,
            ..
        } = self;
        drop(tx);
        let mut summary = task.await.unwrap_or(RecordSummary {
            session_id,
            turns_stored: 0,
            unanalysed_turns: 0,
        });
        if !cancelled && let Err(error) = analyzer.flush(cancel).await {
            tracing::warn!(%error, "the final analysis did not finish");
        }
        summary.unanalysed_turns = analyzer.flagged_turn_ids().len() + analyzer.waiting();
        summary
    }
}

/// Where a stored learner turn is.
struct Remembered {
    turn: u64,
    turn_id: i64,
    voice: bool,
}

async fn record_loop(
    mut rx: mpsc::Receiver<RecordMsg>,
    recording: Recording,
    analyzer: TurnAnalyzer,
    session_id: i64,
    events: EventSink,
) -> RecordSummary {
    let mut analyses: JoinSet<()> = JoinSet::new();
    let cancel = CancellationToken::new();
    let mut last_tutor = String::new();
    let mut stored = 0_usize;
    let mut remembered: VecDeque<Remembered> = VecDeque::new();
    while let Some(message) = rx.recv().await {
        while analyses.try_join_next().is_some() {}
        match message {
            RecordMsg::Opening { tutor_text } => {
                if append(
                    &recording,
                    session_id,
                    TurnRole::Tutor,
                    InputMode::None,
                    &tutor_text,
                    None,
                    None,
                )
                .await
                .is_some()
                {
                    stored += 1;
                }
                last_tutor = tutor_text;
            }
            RecordMsg::Edit { turn, text, reply } => {
                let outcome = edit_stored(&recording, &analyzer, &remembered, turn, &text).await;
                let _ = reply.send(outcome);
            }
            RecordMsg::Turn {
                turn,
                learner_text,
                original,
                input,
                speech_ms,
                tutor_reply,
            } => {
                let mode = match input {
                    AnalysisInput::Voice => InputMode::Voice,
                    AnalysisInput::Text => InputMode::Text,
                };
                let learner = append(
                    &recording,
                    session_id,
                    TurnRole::Learner,
                    mode,
                    &learner_text,
                    original.as_deref(),
                    speech_ms,
                )
                .await;
                if let Some((turn_id, seq)) = learner {
                    stored += 1;
                    remembered.push_back(Remembered {
                        turn,
                        turn_id,
                        voice: input == AnalysisInput::Voice,
                    });
                    while remembered.len() > REMEMBERED_TURNS {
                        remembered.pop_front();
                    }
                    events.publish(VoiceEvent::Recorded {
                        turn,
                        learner_seq: seq,
                        text: learner_text.clone(),
                    });
                }
                if !tutor_reply.is_empty()
                    && append(
                        &recording,
                        session_id,
                        TurnRole::Tutor,
                        InputMode::None,
                        &tutor_reply,
                        None,
                        None,
                    )
                    .await
                    .is_some()
                {
                    stored += 1;
                }
                if let Some((turn_id, turn_seq)) = learner {
                    let turn = TurnToAnalyse {
                        turn_id,
                        turn_seq,
                        input_mode: input,
                        tutor_before: std::mem::take(&mut last_tutor),
                        learner_text,
                        tutor_reply: tutor_reply.clone(),
                    };
                    // Queued here, on the recorder's task, so a correction that comes
                    // next finds the turn waiting; the call itself runs on its own task.
                    analyzer.enqueue(turn);
                    let analyzer = analyzer.clone();
                    let cancel = cancel.clone();
                    let events = events.clone();
                    analyses.spawn(async move {
                        match analyzer.run_due(&cancel).await {
                            Ok(report) => events.publish(VoiceEvent::Analysed(Box::new(report))),
                            Err(error) => tracing::warn!(%error, "turn analysis failed"),
                        }
                    });
                }
                last_tutor = tutor_reply;
            }
        }
    }
    while analyses.join_next().await.is_some() {}
    RecordSummary {
        session_id,
        turns_stored: stored,
        unanalysed_turns: 0,
    }
}

/// Corrects the stored text of a spoken turn and, when its analysis has not
/// started, makes the analysis use the correction.
async fn edit_stored(
    recording: &Recording,
    analyzer: &TurnAnalyzer,
    remembered: &VecDeque<Remembered>,
    turn: u64,
    text: &str,
) -> Result<EditEffect, VoiceError> {
    let found = remembered
        .iter()
        .find(|r| r.turn == turn)
        .ok_or(VoiceError::NoSuchTurn)?;
    if !found.voice {
        return Err(VoiceError::NotSpoken);
    }
    recording
        .db
        .turns()
        .correct_text(found.turn_id, text, Some(words(text)))
        .await?;
    Ok(if analyzer.amend_waiting(found.turn_id, text) {
        EditEffect::AnalysedFromEdit
    } else {
        EditEffect::StoredOnly
    })
}

/// Appends one turn and returns its id and sequence number, or `None` when the
/// database refused it. A failed write is logged and the session goes on.
/// `original` is the recogniser's text when `text` is the learner's correction.
async fn append(
    recording: &Recording,
    session_id: i64,
    role: TurnRole,
    input_mode: InputMode,
    text: &str,
    original: Option<&str>,
    speech_ms: Option<i64>,
) -> Option<(i64, i64)> {
    let result = recording
        .db
        .turns()
        .append(&NewTurn {
            session_id,
            role,
            input_mode,
            text: text.to_owned(),
            stt_text: original.map(str::to_owned),
            edited_by_learner: original.is_some(),
            speech_ms,
            pause_ms: None,
            word_count: Some(words(text)),
            created_at: (recording.clock)(),
        })
        .await;
    match result {
        Ok(turn) => Some((turn.id, turn.seq)),
        Err(error) => {
            tracing::warn!(%error, "a turn could not be stored");
            None
        }
    }
}

/// Closes the stored session row.
pub(crate) async fn close_session(
    recording: &Recording,
    session_id: i64,
    cancelled: bool,
) -> VoiceResult<()> {
    let status = if cancelled {
        SessionStatus::Aborted
    } else {
        SessionStatus::Completed
    };
    recording
        .db
        .sessions()
        .finish(session_id, status, &(recording.clock)(), None)
        .await?;
    Ok(())
}
