//! The background analysis of learner turns (contract T2, ADR-010).
//!
//! The tutor's reply is plain streamed text; this service runs the separate
//! structured call that records what was wrong with the learner's turn. It is a
//! plain async service: the caller spawns `turn_finished` on the runtime and
//! goes on speaking, and nothing here is awaited on the way to the learner's ears.
//!
//! Cadence. Every turn is analysed as soon as the tutor's reply has finished.
//! After a 429 the service switches to batched cadence for the rest of the
//! session: it waits until three turns are queued and sends them in one call,
//! and `flush` sends the rest when the session ends. A turn is never dropped
//! because analysis was late. A turn whose analysis fails stays stored without an
//! analysis, is flagged, and goes into the next batch; after repeated bad
//! output it stays flagged for good and the session summary says so.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use assessment_engine::Level;
use llm_client::{ChatMessage, Contract, LlmClient, LlmError, StructuredRequest};
use storage::{Database, LlmCallType, NewErrorEvent, Timestamp, TurnAnalysis};
use tokio_util::sync::CancellationToken;

use crate::error::{EngineError, Result};
use crate::support::{CallLog, Clock};

use super::filter::{
    AnalysedTurn, DropWindow, ErrorFinding, EvidenceFinding, FilterContext, Notes, RawAnalysis,
    apply_filters,
};
use super::prompt::{
    AnalysisKind, InputMode, ObjectiveRef, TURN_ANALYSIS_VERSION, TurnInput, system_prompt,
    user_message,
};

/// In batched cadence, analysis runs when this many turns are waiting.
pub const BATCH_EVERY: usize = 3;
/// Turns sent in one call, so one request stays small enough to answer in time.
pub const MAX_BATCH_TURNS: usize = 6;
/// Calls made by one `turn_finished`, so a long backlog never keeps one task
/// busy against a provider that is already complaining.
const MAX_BATCHES_PER_CALL: usize = 4;
/// Bad outputs (not outages) a turn may cause before it is left unanalysed.
pub const MAX_HARD_FAILURES: u32 = 3;
/// Turns waiting at once, at most. When the provider is away for very long the
/// oldest waiting turn is given up and stays flagged. This only bounds memory.
pub const MAX_QUEUE: usize = 200;

/// The setting that marks a provider profile "analysis unreliable".
pub const UNRELIABLE_SETTING: &str = "diagnostics.analysis_unreliable";

/// Everything that stays the same for the analyses of one session.
#[derive(Debug, Clone)]
pub struct AnalyzerConfig {
    pub profile_id: i64,
    pub session_id: i64,
    pub provider_profile_id: Option<i64>,
    /// The model name, stored with every analysis (contract rule 5).
    pub model: String,
    /// The level the learner is judged at: the unit's level, or the level the
    /// learner picked for a free mode. Never a value a model produced.
    pub level: Level,
    /// The first language written in English ("Indonesian").
    pub first_language: String,
    pub kind: AnalysisKind,
    pub objectives: Vec<ObjectiveRef>,
    pub target_language: Vec<String>,
}

/// A learner turn that is ready to be analysed: the tutor's reply is complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnToAnalyse {
    pub turn_id: i64,
    pub turn_seq: i64,
    pub input_mode: InputMode,
    /// The tutor's message before the learner's turn. Empty at the start.
    pub tutor_before: String,
    pub learner_text: String,
    /// The tutor's reply to the learner's turn. Empty for a writing draft.
    pub tutor_reply: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cadence {
    /// One call per turn.
    Every,
    /// One call per three turns, after a 429.
    Batched,
}

/// Why a run stopped early.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisFailure {
    RateLimited,
    ProviderUnavailable,
    InvalidOutput,
    Cancelled,
    Other,
}

impl AnalysisFailure {
    fn of(error: &LlmError) -> Self {
        match error {
            LlmError::RateLimited { .. } => Self::RateLimited,
            LlmError::Timeout(_) | LlmError::Transport(_) | LlmError::Server { .. } => {
                Self::ProviderUnavailable
            }
            LlmError::InvalidOutput(_) | LlmError::Refusal => Self::InvalidOutput,
            LlmError::Cancelled => Self::Cancelled,
            _ => Self::Other,
        }
    }

    /// An outage or a limit says nothing about the turn; bad output might.
    fn is_transient(self) -> bool {
        matches!(
            self,
            Self::RateLimited | Self::ProviderUnavailable | Self::Cancelled
        )
    }
}

/// One turn that was analysed and stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysedRecord {
    pub turn_id: i64,
    pub turn_seq: i64,
    /// After the filters.
    pub analysis: AnalysedTurn,
    /// Entries a filter removed.
    pub dropped: usize,
    /// Stored error events, in the order of `analysis.errors`.
    pub error_event_ids: Vec<i64>,
}

impl AnalysedRecord {
    pub fn errors(&self) -> &[ErrorFinding] {
        &self.analysis.errors
    }
    pub fn evidence(&self) -> &[EvidenceFinding] {
        &self.analysis.objective_evidence
    }
}

/// What a call to the service did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunReport {
    pub analysed: Vec<AnalysedRecord>,
    /// Turns still waiting for a call.
    pub waiting: usize,
    pub cadence: Cadence,
    /// Set when a call failed and the run stopped.
    pub failure: Option<AnalysisFailure>,
}

struct Queued {
    turn: TurnToAnalyse,
    /// Any failed analysis, an outage included: this is the "flagged" mark.
    flagged: bool,
    hard_failures: u32,
}

struct State {
    queue: VecDeque<Queued>,
    /// Turns whose text a call has been sent for and not yet settled. Their text
    /// can no longer be amended, because the model already has it.
    in_flight: HashSet<i64>,
    cadence: Cadence,
    notes: Notes,
    window: DropWindow,
    unreliable: bool,
    /// Turns given up on: too many bad outputs, or pushed out of a full queue.
    abandoned: Vec<i64>,
}

struct Inner {
    config: AnalyzerConfig,
    client: Arc<dyn LlmClient>,
    db: Database,
    clock: Clock,
    state: Mutex<State>,
    /// Runs one at a time so the queue is read and written in order.
    run_lock: tokio::sync::Mutex<()>,
}

/// The analysis service of one session. Cloning shares the same queue.
#[derive(Clone)]
pub struct TurnAnalyzer {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for TurnAnalyzer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TurnAnalyzer").finish_non_exhaustive()
    }
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    // A poisoned lock only means another task panicked while holding it; the
    // queue itself is still a valid queue.
    state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl TurnAnalyzer {
    pub fn new(
        config: AnalyzerConfig,
        client: Arc<dyn LlmClient>,
        db: Database,
        clock: Clock,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                config,
                client,
                db,
                clock,
                state: Mutex::new(State {
                    queue: VecDeque::new(),
                    in_flight: HashSet::new(),
                    cadence: Cadence::Every,
                    notes: Notes::default(),
                    window: DropWindow::default(),
                    unreliable: false,
                    abandoned: Vec::new(),
                }),
                run_lock: tokio::sync::Mutex::new(()),
            }),
        }
    }

    /// The latest notes for the next tutor turn, newest first, at most three.
    pub fn notes(&self) -> Vec<String> {
        lock(&self.inner.state).notes.latest()
    }

    pub fn cadence(&self) -> Cadence {
        lock(&self.inner.state).cadence
    }

    /// Turns that are stored without an analysis because a call for them failed:
    /// the ones still waiting for a retry and the ones given up on, in turn order.
    pub fn flagged_turn_ids(&self) -> Vec<i64> {
        let state = lock(&self.inner.state);
        let mut ids: Vec<i64> = state
            .queue
            .iter()
            .filter(|q| q.flagged)
            .map(|q| q.turn.turn_id)
            .chain(state.abandoned.iter().copied())
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    /// Turns waiting for a call, flagged or not.
    pub fn waiting(&self) -> usize {
        lock(&self.inner.state).queue.len()
    }

    /// Replaces the learner's text of a turn that is still waiting for its
    /// analysis, so that the analysis is of the corrected words. Returns `false`
    /// when the turn is not waiting (it was analysed already, or a call for it is
    /// on its way): its analysis is of the text as it was then, and the caller
    /// must say so.
    pub fn amend_waiting(&self, turn_id: i64, learner_text: &str) -> bool {
        let mut state = lock(&self.inner.state);
        if state.in_flight.contains(&turn_id) {
            return false;
        }
        match state.queue.iter_mut().find(|q| q.turn.turn_id == turn_id) {
            Some(queued) => {
                learner_text.clone_into(&mut queued.turn.learner_text);
                true
            }
            None => false,
        }
    }

    /// Whether the filters removed more than a third of the entries of the last
    /// 20 analyses.
    pub fn unreliable(&self) -> bool {
        lock(&self.inner.state).unreliable
    }

    /// Queues a finished turn and, when the cadence says so, analyses what is
    /// waiting. Spawn it; do not await it on the speech path.
    pub async fn turn_finished(
        &self,
        turn: TurnToAnalyse,
        cancel: &CancellationToken,
    ) -> Result<RunReport> {
        {
            let mut state = lock(&self.inner.state);
            state.queue.push_back(Queued {
                turn,
                flagged: false,
                hard_failures: 0,
            });
            while state.queue.len() > MAX_QUEUE {
                if let Some(oldest) = state.queue.pop_front() {
                    state.abandoned.push(oldest.turn.turn_id);
                }
            }
        }
        self.run(false, cancel).await
    }

    /// Analyses everything that is waiting. Call it when the session ends.
    pub async fn flush(&self, cancel: &CancellationToken) -> Result<RunReport> {
        self.run(true, cancel).await
    }

    /// Analyses one turn at once, outside the cadence and the queue, and returns
    /// the stored result. A writing draft uses this; when it fails, the caller
    /// decides what to do (the workshop keeps the draft pending).
    pub async fn analyse_now(
        &self,
        turn: TurnToAnalyse,
        cancel: &CancellationToken,
    ) -> Result<AnalysedRecord> {
        let _guard = self.inner.run_lock.lock().await;
        let mut records = self.call_and_store(&[turn], cancel).await?;
        records
            .pop()
            .ok_or(EngineError::Output("turn_analysis (no turn in the reply)"))
    }

    async fn run(&self, drain: bool, cancel: &CancellationToken) -> Result<RunReport> {
        let _guard = self.inner.run_lock.lock().await;
        let mut analysed = Vec::new();
        let mut failure = None;
        let mut calls = 0;
        loop {
            if !drain && calls == MAX_BATCHES_PER_CALL {
                break;
            }
            let Some(batch) = self.next_batch(drain) else {
                break;
            };
            calls += 1;
            let result = self.call_and_store(&batch, cancel).await;
            lock(&self.inner.state).in_flight.clear();
            match result {
                Ok(records) => {
                    let covered = records.len() >= batch.len();
                    self.settle_success(&batch, &records);
                    analysed.extend(records);
                    if !covered {
                        // A turn the reply left out would be sent again by the next
                        // loop at once; stop and let the next trigger retry it.
                        failure = Some(AnalysisFailure::InvalidOutput);
                        break;
                    }
                }
                Err(EngineError::Llm(error)) => {
                    failure = Some(self.settle_failure(&batch, AnalysisFailure::of(&error)));
                    break;
                }
                Err(EngineError::Output(_)) => {
                    failure = Some(self.settle_failure(&batch, AnalysisFailure::InvalidOutput));
                    break;
                }
                Err(other) => return Err(other),
            }
        }
        let state = lock(&self.inner.state);
        Ok(RunReport {
            analysed,
            waiting: state.queue.len(),
            cadence: state.cadence,
            failure,
        })
    }

    /// The next batch to send, or `None` when the cadence says to wait. Turns
    /// stay in the queue until their analysis is stored.
    fn next_batch(&self, drain: bool) -> Option<Vec<TurnToAnalyse>> {
        let mut state = lock(&self.inner.state);
        let due = match state.cadence {
            Cadence::Every => !state.queue.is_empty(),
            Cadence::Batched => {
                (drain && !state.queue.is_empty()) || state.queue.len() >= BATCH_EVERY
            }
        };
        if !due {
            return None;
        }
        // One call has one input mode, so a typed message in a voice session
        // goes in a call of its own.
        let mode = state.queue.front()?.turn.input_mode;
        let batch: Vec<TurnToAnalyse> = state
            .queue
            .iter()
            .filter(|q| q.turn.input_mode == mode)
            .take(MAX_BATCH_TURNS)
            .map(|q| q.turn.clone())
            .collect();
        state.in_flight = batch.iter().map(|t| t.turn_id).collect();
        Some(batch)
    }

    /// Removes analysed turns from the queue and marks the ones the reply left
    /// out. A turn the model skipped counts as a bad output for that turn.
    fn settle_success(&self, sent: &[TurnToAnalyse], records: &[AnalysedRecord]) {
        let done: HashSet<i64> = records.iter().map(|r| r.turn_id).collect();
        let sent_ids: HashSet<i64> = sent
            .iter()
            .map(|t| t.turn_id)
            .filter(|id| !done.contains(id))
            .collect();
        let mut state = lock(&self.inner.state);
        let mut given_up = Vec::new();
        for queued in state
            .queue
            .iter_mut()
            .filter(|q| sent_ids.contains(&q.turn.turn_id))
        {
            queued.flagged = true;
            queued.hard_failures += 1;
        }
        state.queue.retain(|q| {
            let give_up =
                sent_ids.contains(&q.turn.turn_id) && q.hard_failures >= MAX_HARD_FAILURES;
            if give_up {
                given_up.push(q.turn.turn_id);
            }
            !give_up
        });
        state.abandoned.extend(given_up);
    }

    fn settle_failure(&self, sent: &[TurnToAnalyse], failure: AnalysisFailure) -> AnalysisFailure {
        let sent_ids: HashSet<i64> = sent.iter().map(|t| t.turn_id).collect();
        let mut state = lock(&self.inner.state);
        if failure == AnalysisFailure::RateLimited {
            state.cadence = Cadence::Batched;
        }
        for queued in state
            .queue
            .iter_mut()
            .filter(|q| sent_ids.contains(&q.turn.turn_id))
        {
            queued.flagged = true;
            if !failure.is_transient() {
                queued.hard_failures += 1;
            }
        }
        let mut given_up = Vec::new();
        state.queue.retain(|q| {
            let give_up =
                sent_ids.contains(&q.turn.turn_id) && q.hard_failures >= MAX_HARD_FAILURES;
            if give_up {
                given_up.push(q.turn.turn_id);
            }
            !give_up
        });
        state.abandoned.extend(given_up);
        tracing::debug!(?failure, "turn analysis call failed");
        failure
    }

    fn max_tokens(&self, turns: usize) -> u32 {
        // Starting values. A reply is a small JSON object per turn; a draft may
        // carry twenty errors, each with a quote and a correction.
        let (base, per_turn) = match self.inner.config.kind {
            AnalysisKind::Turn => (300u32, 400u32),
            AnalysisKind::Draft => (400, 2000),
        };
        base + per_turn * u32::try_from(turns).unwrap_or(1)
    }

    /// One call for `turns`, then the filters and the storage. Returns the
    /// records of the turns the reply covered.
    async fn call_and_store(
        &self,
        turns: &[TurnToAnalyse],
        cancel: &CancellationToken,
    ) -> Result<Vec<AnalysedRecord>> {
        let Some(first) = turns.first() else {
            return Ok(Vec::new());
        };
        let config = &self.inner.config;
        let mode = if config.kind == AnalysisKind::Draft {
            InputMode::Text
        } else {
            first.input_mode
        };
        let inputs: Vec<TurnInput> = turns
            .iter()
            .map(|t| TurnInput {
                turn_seq: t.turn_seq,
                tutor_before: t.tutor_before.clone(),
                learner_text: t.learner_text.clone(),
                tutor_reply: t.tutor_reply.clone(),
            })
            .collect();
        let message = user_message(
            config.level,
            &config.first_language,
            mode,
            &config.objectives,
            &config.target_language,
            &inputs,
        );
        let request = StructuredRequest::new(
            Contract::TurnAnalysis,
            system_prompt(config.kind),
            vec![ChatMessage::user(message)],
            self.max_tokens(turns.len()),
        )
        .with_temperature(0.0);

        let started_at = (self.inner.clock)();
        let timer = Instant::now();
        let result = self.inner.client.structured(request, cancel.clone()).await;
        CallLog {
            db: &self.inner.db,
            provider_profile_id: config.provider_profile_id,
            call_type: LlmCallType::TurnAnalysis,
            model: &config.model,
            started_at,
            elapsed: timer.elapsed(),
        }
        .structured(&result)
        .await;
        let output = result?;

        let raw: RawAnalysis = serde_json::from_value(output.value)
            .map_err(|_| EngineError::Output("turn_analysis"))?;
        let ladder = i64::from(output.ladder_level.as_u8());
        self.store_all(turns, mode, raw, ladder).await
    }

    async fn store_all(
        &self,
        turns: &[TurnToAnalyse],
        mode: InputMode,
        raw: RawAnalysis,
        ladder_level: i64,
    ) -> Result<Vec<AnalysedRecord>> {
        let config = &self.inner.config;
        let objective_ids: HashSet<String> =
            config.objectives.iter().map(|o| o.id.clone()).collect();
        let by_seq: HashMap<i64, &TurnToAnalyse> = turns.iter().map(|t| (t.turn_seq, t)).collect();
        let mut stored_seqs: HashSet<i64> = HashSet::new();
        let mut records = Vec::new();
        let mut reply_turns = raw.turns;
        reply_turns.sort_by_key(|t| t.turn_seq);

        for reply in reply_turns {
            // A turn_seq we did not send, or one the reply repeats, is noise.
            let Some(turn) = by_seq.get(&reply.turn_seq).copied() else {
                continue;
            };
            if !stored_seqs.insert(reply.turn_seq) {
                continue;
            }
            let filtered = apply_filters(
                &reply,
                &FilterContext {
                    learner_text: &turn.learner_text,
                    mode,
                    objective_ids: &objective_ids,
                    max_errors: config.kind.max_errors(),
                },
            );
            let now = (self.inner.clock)();
            let record = self
                .store_turn(turn, filtered.turn, filtered.dropped, ladder_level, now)
                .await?;
            // Out of the queue as soon as it is stored, so a later failure in this
            // batch cannot make the turn be analysed twice.
            lock(&self.inner.state)
                .queue
                .retain(|q| q.turn.turn_id != turn.turn_id);
            self.after_stored(&record, filtered.entries).await;
            records.push(record);
        }
        Ok(records)
    }

    async fn store_turn(
        &self,
        turn: &TurnToAnalyse,
        analysis: AnalysedTurn,
        dropped: usize,
        ladder_level: i64,
        now: Timestamp,
    ) -> Result<AnalysedRecord> {
        let config = &self.inner.config;
        let document = RawAnalysis {
            turns: vec![analysis.clone()],
        };
        let stored = TurnAnalysis {
            turn_id: turn.turn_id,
            analysis: serde_json::to_value(&document)
                .map_err(|_| EngineError::Output("turn_analysis"))?,
            contract_version: TURN_ANALYSIS_VERSION.to_owned(),
            ladder_level,
            model: config.model.clone(),
            created_at: now,
        };
        let events: Vec<NewErrorEvent> = analysis
            .errors
            .iter()
            .map(|e| NewErrorEvent {
                turn_id: turn.turn_id,
                profile_id: config.profile_id,
                category: e.category.clone(),
                quote: e.quote.clone(),
                correction: e.correction.clone(),
                severity: e.severity,
                created_at: now,
            })
            .collect();
        let repo = self.inner.db.analysis();
        repo.store(&stored, &events).await?;

        let mut counts: Vec<(&str, u32)> = Vec::new();
        for error in &analysis.errors {
            match counts.iter_mut().find(|(c, _)| *c == error.category) {
                Some((_, n)) => *n += 1,
                None => counts.push((error.category.as_str(), 1)),
            }
        }
        for (category, count) in counts {
            self.inner
                .db
                .error_stats()
                .add(config.profile_id, category, count, &now)
                .await?;
        }

        let stored_events = repo.error_events(turn.turn_id).await?;
        // `store` adds events and a re-analysed turn keeps the earlier ones, so
        // the ids of this analysis are the last ones, in order.
        let start = stored_events.len().saturating_sub(analysis.errors.len());
        let ids: Vec<i64> = stored_events[start..].iter().map(|e| e.id).collect();
        for (id, error) in ids.iter().zip(&analysis.errors) {
            if error.addressed_in_reply {
                repo.mark_addressed(*id).await?;
            }
        }
        Ok(AnalysedRecord {
            turn_id: turn.turn_id,
            turn_seq: turn.turn_seq,
            analysis,
            dropped,
            error_event_ids: ids,
        })
    }

    /// Notes and the reliability mark, once a turn is stored.
    async fn after_stored(&self, record: &AnalysedRecord, entries: usize) {
        let flip = {
            let mut state = lock(&self.inner.state);
            state.notes.push(&record.analysis.note_for_next_turn);
            state.window.record(entries, record.dropped);
            let now_unreliable = state.window.unreliable();
            let changed = now_unreliable != state.unreliable;
            state.unreliable = now_unreliable;
            changed.then_some(now_unreliable)
        };
        if let Some(value) = flip {
            let key = unreliable_key(self.inner.config.provider_profile_id);
            let stamp = (self.inner.clock)();
            let text = if value { "1" } else { "0" };
            if let Err(error) = self.inner.db.settings().set(&key, text, &stamp).await {
                tracing::warn!(%error, "could not store the analysis reliability mark");
            }
        }
    }
}

fn unreliable_key(provider_profile_id: Option<i64>) -> String {
    match provider_profile_id {
        Some(id) => format!("{UNRELIABLE_SETTING}.{id}"),
        None => UNRELIABLE_SETTING.to_owned(),
    }
}

/// Whether the diagnostics page should say "analysis unreliable" for a provider
/// profile (contract T2).
pub async fn analysis_unreliable(db: &Database, provider_profile_id: Option<i64>) -> Result<bool> {
    let value = db
        .settings()
        .get(&unreliable_key(provider_profile_id))
        .await?;
    Ok(value.as_deref() == Some("1"))
}
