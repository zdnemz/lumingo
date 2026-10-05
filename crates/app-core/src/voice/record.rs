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

use std::sync::Arc;

use llm_client::LlmClient;
use storage::{Database, InputMode, NewSession, NewTurn, SessionMode, SessionStatus, TurnRole};
use tokio::sync::mpsc;
use tokio::task::{JoinHandle, JoinSet};
use tokio_util::sync::CancellationToken;
use tutor_engine::{
    AnalysisKind, AnalyzerConfig, Clock, FeedbackMode, InputMode as AnalysisInput, TurnAnalyzer,
    TurnToAnalyse,
};

use super::config::Scenario;
use super::error::VoiceResult;
use super::msg::Counters;

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
    pub clock: Clock,
}

#[derive(Debug)]
pub(crate) enum RecordMsg {
    /// The tutor spoke first.
    Opening { tutor_text: String },
    Turn {
        learner_text: String,
        input: AnalysisInput,
        speech_ms: Option<i64>,
        tutor_reply: String,
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
    ) -> VoiceResult<Self> {
        let stored = recording
            .db
            .sessions()
            .create(&NewSession {
                profile_id: recording.profile_id,
                kind: storage::SessionKind::Conversation,
                unit_id: scenario.unit_id.clone(),
                activity_id: scenario.activity_id.clone(),
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
        let task = tokio::spawn(record_loop(rx, recording, analyzer.clone(), stored.id));
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

async fn record_loop(
    mut rx: mpsc::Receiver<RecordMsg>,
    recording: Recording,
    analyzer: TurnAnalyzer,
    session_id: i64,
) -> RecordSummary {
    let mut analyses: JoinSet<()> = JoinSet::new();
    let cancel = CancellationToken::new();
    let mut last_tutor = String::new();
    let mut stored = 0_usize;
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
                )
                .await
                .is_some()
                {
                    stored += 1;
                }
                last_tutor = tutor_text;
            }
            RecordMsg::Turn {
                learner_text,
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
                    speech_ms,
                )
                .await;
                if learner.is_some() {
                    stored += 1;
                }
                if !tutor_reply.is_empty()
                    && append(
                        &recording,
                        session_id,
                        TurnRole::Tutor,
                        InputMode::None,
                        &tutor_reply,
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
                    let analyzer = analyzer.clone();
                    let cancel = cancel.clone();
                    analyses.spawn(async move {
                        if let Err(error) = analyzer.turn_finished(turn, &cancel).await {
                            tracing::warn!(%error, "turn analysis failed");
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

/// Appends one turn and returns its id and sequence number, or `None` when the
/// database refused it. A failed write is logged and the session goes on.
async fn append(
    recording: &Recording,
    session_id: i64,
    role: TurnRole,
    input_mode: InputMode,
    text: &str,
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
            stt_text: None,
            edited_by_learner: false,
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
