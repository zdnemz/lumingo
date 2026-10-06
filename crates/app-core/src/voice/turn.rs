//! One tutor turn: the language model's reply, cut into sentences, spoken.
//!
//! A turn is an async task on the Tokio runtime. It does no inference and no
//! blocking work: the model is a remote call, and speech runs on the TTS
//! worker's own thread.
//!
//! # Flow
//!
//! 1. Open the reply stream and wait for its first event. This is one attempt,
//!    limited to half of `VoiceConfig::provider_timeout`. A failed attempt is
//!    tried once more; a second failure ends the turn as `ProviderUnavailable`.
//!    The retry is only ever made before any text reached the learner, because
//!    a repeated reply would repeat spoken words.
//! 2. Feed every delta to the sentence chunker. Each finished sentence goes to
//!    the TTS worker at once, so the first sentence is synthesised while the
//!    rest of the reply is still arriving. A worker queue that is full (8
//!    sentences) hands the sentence back; it waits in a local backlog and is
//!    offered again on the next poll. Nothing is dropped.
//! 3. The worker's audio is queued for playback by the speaker sink. The turn
//!    reads small notes about it, stamps the latency and, when the output
//!    callback first consumes the audio, moves the session to `Speaking`.
//! 4. When the stream has ended and every sentence has audio, the turn tells
//!    the playback queue the reply is complete, waits for it to drain, and
//!    moves the session back to `Listening`.
//!
//! A reply that is empty or refused is replaced once by the authored line;
//! a second empty reply in a row is treated as a provider failure, as in the
//! text chat. A stream that breaks after some text has arrived keeps that text:
//! the words are already spoken.
//!
//! # Cancellation
//!
//! The turn watches its `CancellationToken` in every wait. When it fires the
//! turn cancels its TTS turn (so queued sentences end without being
//! synthesised) and returns. Flushing playback is the orchestrator's job and
//! happens before the token fires.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use llm_client::{LlmClient, LlmError, StreamEvent, TextRequest, TextStream, TimeoutKind};
use speech::{TtsSubmitError, TtsTurn, TtsWorker};
use tokio::sync::mpsc;
use tokio::time::{Instant, Interval, MissedTickBehavior};
use tokio_util::sync::CancellationToken;
use tutor_engine::{EngineFault, Event, FALLBACK_LINE, SentenceChunker};

use super::clock::LoopClock;
use super::config::VoiceConfig;
use super::event::{EventSink, TurnOutcome, VoiceEvent};
use super::latency::{LatencyParts, Stamps, TurnLatency};
use super::phase::PhaseCell;
use super::port::PlaybackPort;
use super::speaker::TtsNote;

/// What a turn says.
pub(crate) enum Plan {
    /// Ask the model.
    Model(TextRequest),
    /// Speak this line without asking the model.
    Canned(String),
}

/// Speech output: the worker that synthesises and the queue that plays.
#[derive(Clone)]
pub(crate) struct SpeechOut {
    pub tts: Arc<TtsWorker>,
    pub playback: Arc<dyn PlaybackPort>,
}

/// What every turn shares.
#[derive(Clone)]
pub(crate) struct TurnEnv {
    pub llm: Arc<dyn LlmClient>,
    pub clock: Arc<dyn LoopClock>,
    pub cell: Arc<PhaseCell>,
    pub events: EventSink,
    pub speech: Option<SpeechOut>,
    pub config: Arc<VoiceConfig>,
}

pub(crate) struct TurnRun {
    pub epoch: u64,
    pub turn: u64,
    pub plan: Plan,
    /// What is known when the turn starts: the instants up to the transcript.
    pub stamps: Stamps,
    /// Whether an empty reply may be answered with the authored line.
    pub allow_fallback: bool,
    pub token: CancellationToken,
    pub notes: mpsc::Receiver<TtsNote>,
}

/// What the turn hands back to the orchestrator.
#[derive(Debug)]
pub(crate) struct TurnReport {
    pub epoch: u64,
    pub turn: u64,
    pub outcome: TurnOutcome,
    /// The reply as it was produced, for the history and the record.
    pub reply: String,
    pub latency: Option<TurnLatency>,
    /// For the learner, when the turn ended in a failure.
    pub message: Option<String>,
    pub fault: Option<EngineFault>,
}

enum Opened {
    Ready(TextStream, StreamEvent),
    Cancelled,
    Failed(Option<LlmError>),
}

fn retryable(error: &LlmError) -> bool {
    error.is_provider_unavailable()
        || matches!(error, LlmError::Stream { .. } | LlmError::Protocol(_))
}

/// The sentence the learner sees when the provider is out of reach. It names the
/// failure by its category only: provider errors are already free of keys.
pub(crate) fn provider_message(error: Option<&LlmError>) -> String {
    let cause = match error {
        Some(error) => format!(" ({error})"),
        None => String::new(),
    };
    format!(
        "The language model provider could not be reached or kept failing{cause}. \
         Check the connection and the provider settings, then resume."
    )
}

struct Turn<'a> {
    env: &'a TurnEnv,
    epoch: u64,
    turn: u64,
    stamps: Stamps,
    chunker: SentenceChunker,
    reply: String,
    sentences: u32,
    backlog: VecDeque<String>,
    tts_turn: Option<TtsTurn<'a>>,
    submitted: u32,
    finished: u32,
    notes_closed: bool,
    awaiting_output: bool,
    played_base: u64,
    latency: Option<TurnLatency>,
    failure: Option<(EngineFault, String)>,
    tts_deadline: Option<Instant>,
    used_fallback: bool,
    started_reply: bool,
    /// The stream (or the canned line) is over; no more sentences will come.
    ended: bool,
    was_cancelled: bool,
}

pub(crate) async fn run(env: TurnEnv, input: TurnRun) -> TurnReport {
    let TurnRun {
        epoch,
        turn,
        plan,
        stamps,
        allow_fallback,
        token,
        mut notes,
    } = input;
    let mut t = Turn::new(&env, epoch, turn, stamps);

    let mut stream: Option<TextStream> = None;
    let mut first: Option<Result<StreamEvent, LlmError>> = None;
    match plan {
        Plan::Canned(line) => {
            t.used_fallback = true;
            t.reply.clone_from(&line);
            t.on_sentence(line);
            t.ended = true;
        }
        Plan::Model(request) => match t.open(&request, &token).await {
            Opened::Ready(opened, event) => {
                stream = Some(opened);
                first = Some(Ok(event));
            }
            Opened::Cancelled => return t.cancelled(),
            Opened::Failed(error) => return t.provider_unavailable(error.as_ref()),
        },
    }

    let mut tick = tokio::time::interval(env.config.output_poll);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut streaming = stream.is_some();
    let mut broke: Option<LlmError> = None;

    loop {
        if let Some(item) = first.take() {
            match item {
                Ok(StreamEvent::Delta(delta)) => t.on_delta(&delta),
                Ok(StreamEvent::Finished(_)) => {
                    streaming = false;
                    t.on_end();
                }
                Err(error) => {
                    streaming = false;
                    broke = Some(error);
                    t.on_end();
                }
            }
        }
        if t.failure.is_some() {
            break;
        }
        if t.ended {
            if t.sentences == 0 {
                // An empty reply: the authored line once, then the provider is
                // treated as failing.
                if !allow_fallback {
                    return t.provider_unavailable(broke.as_ref());
                }
                t.used_fallback = true;
                t.reply = FALLBACK_LINE.to_owned();
                t.on_sentence(FALLBACK_LINE.to_owned());
                continue;
            }
            if t.reply_done() {
                break;
            }
        }
        tokio::select! {
            biased;
            () = token.cancelled() => return t.cancelled(),
            item = next_item(&mut stream), if streaming => match item {
                Some(item) => first = Some(item),
                None => {
                    streaming = false;
                    t.on_end();
                }
            },
            note = notes.recv(), if t.expects_notes() => match note {
                Some(note) => t.on_note(note),
                None => t.notes_closed = true,
            },
            _ = tick.tick() => t.on_tick(),
        }
    }

    if t.failure.is_none() {
        t.drain(&token, &mut tick).await;
    }
    t.finish(broke.as_ref())
}

async fn next_item(stream: &mut Option<TextStream>) -> Option<Result<StreamEvent, LlmError>> {
    match stream {
        Some(stream) => stream.next().await,
        None => None,
    }
}

impl<'a> Turn<'a> {
    fn new(env: &'a TurnEnv, epoch: u64, turn: u64, stamps: Stamps) -> Self {
        let played_base = env
            .speech
            .as_ref()
            .map_or(0, |s| s.playback.samples_played());
        Self {
            tts_turn: env.speech.as_ref().map(|s| s.tts.turn(epoch)),
            env,
            epoch,
            turn,
            stamps,
            chunker: SentenceChunker::new(),
            reply: String::new(),
            sentences: 0,
            backlog: VecDeque::new(),
            submitted: 0,
            finished: 0,
            notes_closed: false,
            awaiting_output: false,
            played_base,
            latency: None,
            failure: None,
            tts_deadline: None,
            used_fallback: false,
            started_reply: false,
            ended: false,
            was_cancelled: false,
        }
    }

    fn now(&self) -> Duration {
        self.env.clock.now()
    }

    /// One attempt is the call plus its first event, bounded by half the budget.
    async fn open(&self, request: &TextRequest, token: &CancellationToken) -> Opened {
        let per_attempt = self.env.config.provider_timeout / 2;
        let mut last: Option<LlmError> = None;
        for _attempt in 0..2 {
            let call = async {
                let mut stream = self
                    .env
                    .llm
                    .stream_text(request.clone(), token.clone())
                    .await?;
                let first = stream.next().await;
                Ok::<_, LlmError>((stream, first))
            };
            let result = tokio::select! {
                biased;
                () = token.cancelled() => return Opened::Cancelled,
                result = tokio::time::timeout(per_attempt, call) => result,
            };
            let error = match result {
                Ok(Ok((stream, Some(Ok(first))))) => return Opened::Ready(stream, first),
                Ok(Ok((_, Some(Err(error))))) | Ok(Err(error)) => error,
                Ok(Ok((_, None))) => {
                    LlmError::Protocol("the stream ended before any event".to_owned())
                }
                Err(_elapsed) => LlmError::Timeout(TimeoutKind::FirstToken),
            };
            if matches!(error, LlmError::Cancelled) {
                return Opened::Cancelled;
            }
            tracing::warn!(%error, "the provider call failed");
            let again = retryable(&error);
            last = Some(error);
            if !again {
                break;
            }
        }
        Opened::Failed(last)
    }

    fn on_delta(&mut self, delta: &str) {
        self.reply.push_str(delta);
        for sentence in self.chunker.push(delta) {
            self.on_sentence(sentence);
        }
    }

    /// The stream ended: the rest of the text is the last sentence.
    fn on_end(&mut self) {
        if let Some(rest) = self.chunker.finish() {
            self.on_sentence(rest);
        }
        self.ended = true;
    }

    fn on_sentence(&mut self, text: String) {
        let index = self.sentences;
        self.sentences += 1;
        if index == 0 {
            self.stamps.first_sentence = Some(self.now());
        }
        self.env.events.publish(VoiceEvent::TutorSentence {
            turn: self.turn,
            index,
            text: text.clone(),
        });
        if self.env.speech.is_some() {
            self.backlog.push_back(text);
            self.flush_backlog();
        } else if index == 0 {
            // No speech output: the reply starts when its first sentence is ready.
            self.publish_latency();
            self.start_reply();
        }
    }

    /// Offers the waiting sentences to the TTS worker, in order, until it is full.
    fn flush_backlog(&mut self) {
        let Some(tts_turn) = self.tts_turn.as_mut() else {
            return;
        };
        while let Some(text) = self.backlog.pop_front() {
            match tts_turn.speak(text) {
                Ok(_) => {
                    self.submitted += 1;
                    if self.tts_deadline.is_none() {
                        self.tts_deadline = Some(Instant::now() + self.env.config.tts_timeout);
                    }
                }
                Err(TtsSubmitError::Full(job)) => {
                    self.backlog.push_front(job.text);
                    break;
                }
                Err(TtsSubmitError::Stopped(_)) => {
                    self.fail(
                        EngineFault::SpeechSynthesis,
                        "the speech output has stopped",
                    );
                    break;
                }
            }
        }
    }

    fn expects_notes(&self) -> bool {
        !self.notes_closed && self.submitted > self.finished
    }

    fn on_note(&mut self, note: TtsNote) {
        self.finished += 1;
        self.tts_deadline =
            (self.submitted > self.finished).then(|| Instant::now() + self.env.config.tts_timeout);
        match note {
            TtsNote::Audio { at, .. } => {
                if self.stamps.audio_queued.is_none() {
                    self.stamps.audio_queued = Some(at);
                    self.awaiting_output = true;
                }
            }
            TtsNote::Failed { message, .. } => {
                self.fail(EngineFault::SpeechSynthesis, &message);
            }
            TtsNote::Cancelled => {}
        }
        self.flush_backlog();
        self.check_output();
    }

    fn on_tick(&mut self) {
        self.flush_backlog();
        self.check_output();
        let overdue = self
            .tts_deadline
            .is_some_and(|deadline| Instant::now() >= deadline);
        if overdue && self.submitted > self.finished {
            self.fail(
                EngineFault::SpeechSynthesis,
                "speech synthesis took too long",
            );
        }
    }

    /// True when every sentence has been handed over, synthesised and queued.
    fn reply_done(&self) -> bool {
        self.backlog.is_empty() && self.finished >= self.submitted
    }

    fn fail(&mut self, fault: EngineFault, message: &str) {
        if self.failure.is_none() {
            self.failure = Some((fault, message.to_owned()));
        }
    }

    fn check_output(&mut self) {
        if !self.awaiting_output {
            return;
        }
        let Some(speech) = &self.env.speech else {
            return;
        };
        if speech.playback.samples_played() > self.played_base {
            self.awaiting_output = false;
            self.stamps.first_sample_played = Some(self.now());
            self.publish_latency();
            self.start_reply();
        }
    }

    fn publish_latency(&mut self) {
        let latency = TurnLatency {
            turn: self.turn,
            parts: LatencyParts::from_stamps(&self.stamps),
        };
        self.latency = Some(latency);
        self.env.events.publish(VoiceEvent::Latency(latency));
    }

    fn start_reply(&mut self) {
        if !self.started_reply {
            self.started_reply = true;
            self.env.cell.apply_for(self.epoch, Event::ReplyStarted);
        }
    }

    /// Waits for the queued audio to play out. The queue is told first that the
    /// reply is complete, so running dry at the end is not counted as an underrun.
    async fn drain(&mut self, token: &CancellationToken, tick: &mut Interval) {
        let Some(speech) = self.env.speech.clone() else {
            return;
        };
        if self.stamps.audio_queued.is_none() {
            return;
        }
        speech.playback.finish_turn();
        let deadline = Instant::now() + speech.playback.queued() + self.env.config.drain_grace;
        while speech.playback.is_active() {
            tokio::select! {
                biased;
                () = token.cancelled() => {
                    self.was_cancelled = true;
                    return;
                }
                _ = tick.tick() => {
                    self.check_output();
                    if Instant::now() >= deadline {
                        self.fail(EngineFault::Playback, "the audio output did not play the reply");
                        return;
                    }
                }
            }
        }
        self.check_output();
    }

    fn cancel_speech(&self) {
        if let Some(turn) = &self.tts_turn {
            turn.cancel();
        }
    }

    fn report(self, outcome: TurnOutcome) -> TurnReport {
        TurnReport {
            epoch: self.epoch,
            turn: self.turn,
            outcome,
            reply: self.reply.trim().to_owned(),
            latency: self.latency,
            message: None,
            fault: None,
        }
    }

    fn cancelled(self) -> TurnReport {
        self.cancel_speech();
        self.report(TurnOutcome::Stopped)
    }

    fn provider_unavailable(self, error: Option<&LlmError>) -> TurnReport {
        self.cancel_speech();
        self.env.cell.apply_for(self.epoch, Event::ProviderFailed);
        let message = provider_message(error);
        let mut report = self.report(TurnOutcome::ProviderUnavailable);
        report.message = Some(message);
        report
    }

    fn finish(self, broke: Option<&LlmError>) -> TurnReport {
        if self.was_cancelled {
            return self.cancelled();
        }
        if let Some((fault, message)) = self.failure.clone() {
            self.cancel_speech();
            let mut report = self.report(TurnOutcome::EngineError);
            report.message = Some(message);
            report.fault = Some(fault);
            return report;
        }
        if let Some(error) = broke {
            // Text that was already spoken stays the reply.
            tracing::warn!(%error, "the reply stream broke after some text had arrived");
        }
        self.env.cell.apply_for(self.epoch, Event::ReplyFinished);
        let outcome = if self.used_fallback {
            TurnOutcome::Fallback
        } else {
            TurnOutcome::Replied
        };
        self.report(outcome)
    }
}
