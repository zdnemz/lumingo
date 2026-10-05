//! The text chat session (S4-11): the conversation engine on the text channel.
//!
//! One tutor, by typing. The scenario comes from the topic bank or from a topic
//! the learner typed. Each message is stored as a turn; the tutor's reply is
//! streamed as plain text (ADR-010) while the analysis of the message runs in
//! the background. Chat is practice only: attempts are stored with origin
//! `free_mode` and never count toward an estimate.

use std::sync::Arc;
use std::time::Instant;

use assessment_engine::Level;
use futures_util::StreamExt;
use llm_client::{
    ChatMessage, FinishReason, LlmClient, LlmError, Role, StreamEvent, StreamSummary, TextRequest,
};
use serde::Serialize;
use serde_json::json;
use storage::{
    AttemptOrigin, AttemptStatus, Database, EvidenceKind, InputMode, LlmCallType, LlmOutcome,
    NewAttempt, NewEvidence, NewSession, NewTurn, Scorer, SessionMode, SessionStatus, Timestamp,
    TurnRole,
};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::analysis::{
    AnalysisKind, AnalyzerConfig, InputMode as AnalysisInput, TurnAnalyzer, TurnToAnalyse,
};
use crate::error::{EngineError, Result};
use crate::prompt::{
    FALLBACK_LINE, FeedbackMode, Focus, HISTORY_MESSAGES, TutorContext, bounded_history,
    reply_limits, system_prompt, user_message,
};
use crate::session::{Channel, EndReason, Event, Phase, Session, SessionKind};
use crate::support::{CallLog, Clock, single_line, storage_level};
use crate::topics::ConversationTopic;

/// Longest typed topic, in characters. It goes into the system prompt, so it is
/// kept to a title.
pub const MAX_TOPIC_CHARS: usize = 80;

/// Where the conversation's scenario comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatTopic {
    /// An entry of the topic bank for the learner's level.
    Bank(ConversationTopic),
    /// A topic the learner typed.
    Typed(String),
}

#[derive(Debug, Clone)]
pub struct ChatConfig {
    pub profile_id: i64,
    pub provider_profile_id: Option<i64>,
    pub model: String,
    /// The level the tutor speaks at: the learner's pick or their current
    /// estimate. Never a value a model produced.
    pub level: Level,
    /// The first language written in English ("Indonesian").
    pub first_language: String,
    pub mode: FeedbackMode,
    pub topic: ChatTopic,
    pub app_version: String,
}

/// What a chat session works with.
#[derive(Clone)]
pub struct ChatDeps {
    pub client: Arc<dyn LlmClient>,
    pub db: Database,
    pub clock: Clock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyOutcome {
    /// The model's reply, in full.
    Normal,
    /// The model said nothing or refused: the authored line was shown instead.
    Fallback,
    /// The reply could not be had; the session is now `ProviderUnavailable`.
    ProviderUnavailable,
    /// The learner stopped the reply. What had arrived is kept; the session is paused.
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatReply {
    pub text: String,
    pub outcome: ReplyOutcome,
    pub learner_turn_id: Option<i64>,
    pub tutor_turn_id: Option<i64>,
}

/// One error category seen in the session, with one example.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ErrorPattern {
    pub category: String,
    pub count: usize,
    pub quote: String,
    pub correction: String,
}

/// The end summary of a chat: what to practise next.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChatSummary {
    pub learner_turns: usize,
    /// Most frequent first, at most five.
    pub top_errors: Vec<ErrorPattern>,
    /// Sequence numbers of learner turns that have no analysis.
    pub unanalysed_turns: Vec<i64>,
    pub analysis_unreliable: bool,
}

pub struct TextChat {
    config: ChatConfig,
    deps: ChatDeps,
    session: Session,
    session_id: i64,
    system: String,
    analyzer: TurnAnalyzer,
    tasks: JoinSet<()>,
    analysis_cancel: CancellationToken,
    last_tutor_text: String,
    empty_streak: u32,
}

/// A typed topic reduced to one safe line: no line breaks, no tag characters.
pub fn clean_topic(topic: &str) -> String {
    let flat = topic.replace(['<', '>'], " ");
    single_line(&flat, MAX_TOPIC_CHARS)
}

fn context(config: &ChatConfig) -> Option<TutorContext> {
    let (focus, scenario, tutor_role, learner_role, goals) = match &config.topic {
        ChatTopic::Bank(topic) => (
            Focus::Topic(topic.title.en.clone()),
            topic.scenario.en.clone(),
            topic.tutor_role.clone(),
            topic.learner_role.clone(),
            topic.goals.clone(),
        ),
        ChatTopic::Typed(text) => {
            let title = clean_topic(text);
            if title.is_empty() {
                return None;
            }
            (
                Focus::Topic(title),
                "an open conversation about this topic".to_owned(),
                "a friendly conversation partner".to_owned(),
                "a conversation partner".to_owned(),
                vec![
                    "keep the conversation going for several turns".to_owned(),
                    "give and ask for opinions or details".to_owned(),
                ],
            )
        }
    };
    Some(TutorContext {
        channel: Channel::Text,
        level: config.level,
        first_language: config.first_language.clone(),
        focus,
        scenario,
        tutor_role,
        learner_role,
        goals,
        target_language: Vec::new(),
        mode: config.mode,
        pronunciation_findings: false,
    })
}

fn session_mode(mode: FeedbackMode) -> SessionMode {
    match mode {
        FeedbackMode::Fluency => SessionMode::Fluency,
        FeedbackMode::Accuracy => SessionMode::Accuracy,
    }
}

/// Turns the stored window into provider messages: the window starts with a
/// learner message, because a provider rejects a conversation that opens with
/// the assistant, and consecutive messages of one role are joined, because a
/// learner message whose reply failed would otherwise leave two in a row.
fn provider_messages(history: &[(TurnRole, String)]) -> Vec<ChatMessage> {
    let mut out: Vec<ChatMessage> = Vec::new();
    for (role, text) in history {
        let (role, content) = match role {
            TurnRole::Learner => (Role::User, user_message(text, &[], &[])),
            TurnRole::Tutor => (Role::Assistant, text.clone()),
        };
        match out.last_mut() {
            Some(last) if last.role == role => {
                last.content.push('\n');
                last.content.push_str(&content);
            }
            _ => out.push(ChatMessage { role, content }),
        }
    }
    while out.first().is_some_and(|m| m.role == Role::Assistant) {
        out.remove(0);
    }
    out
}

impl TextChat {
    /// Creates the stored session and the analysis service.
    pub async fn start(deps: ChatDeps, config: ChatConfig) -> Result<Self> {
        let ctx = context(&config).ok_or(EngineError::Refused("the topic is empty"))?;
        let activity_id = match &config.topic {
            ChatTopic::Bank(topic) => Some(topic.id.clone()),
            ChatTopic::Typed(_) => None,
        };
        let stored = deps
            .db
            .sessions()
            .create(&NewSession {
                profile_id: config.profile_id,
                kind: storage::SessionKind::TextChat,
                unit_id: None,
                activity_id,
                mode: Some(session_mode(config.mode)),
                provider_profile_id: config.provider_profile_id,
                app_version: config.app_version.clone(),
                started_at: (deps.clock)(),
            })
            .await?;
        let analyzer = TurnAnalyzer::new(
            AnalyzerConfig {
                profile_id: config.profile_id,
                session_id: stored.id,
                provider_profile_id: config.provider_profile_id,
                model: config.model.clone(),
                level: config.level,
                first_language: config.first_language.clone(),
                kind: AnalysisKind::Turn,
                objectives: Vec::new(),
                target_language: Vec::new(),
            },
            deps.client.clone(),
            deps.db.clone(),
            deps.clock.clone(),
        );
        Ok(Self {
            system: system_prompt(&ctx),
            session: Session::new(SessionKind::TextChat, Channel::Text),
            session_id: stored.id,
            config,
            deps,
            analyzer,
            tasks: JoinSet::new(),
            analysis_cancel: CancellationToken::new(),
            last_tutor_text: String::new(),
            empty_streak: 0,
        })
    }

    pub fn session_id(&self) -> i64 {
        self.session_id
    }

    pub fn phase(&self) -> Phase {
        self.session.phase()
    }

    /// The latest notes from the analysis, which go into the next tutor turn.
    pub fn notes(&self) -> Vec<String> {
        self.analyzer.notes()
    }

    pub fn analyzer(&self) -> &TurnAnalyzer {
        &self.analyzer
    }

    /// The tutor speaks first. Call it once, before the learner's first message.
    pub async fn open(
        &mut self,
        on_delta: impl FnMut(&str) + Send,
        cancel: &CancellationToken,
    ) -> Result<ChatReply> {
        self.session.apply(Event::TextSent)?;
        let start = ChatMessage::user(
            "Begin the conversation now: greet the learner in your role and ask your first question.",
        );
        self.reply(vec![start], None, on_delta, cancel).await
    }

    /// Handles one learner message: stores it, streams the tutor's reply through
    /// `on_delta` as it arrives, stores the reply, and hands the message to the
    /// background analysis. Returns when the reply is complete; the analysis may
    /// still be running.
    pub async fn send(
        &mut self,
        text: &str,
        on_delta: impl FnMut(&str) + Send,
        cancel: &CancellationToken,
    ) -> Result<ChatReply> {
        let text = text.trim();
        if text.is_empty() {
            return Err(EngineError::Refused("the message is empty"));
        }
        self.session.apply(Event::TextSent)?;
        while self.tasks.try_join_next().is_some() {}

        let now = (self.deps.clock)();
        let words = i64::try_from(text.split_whitespace().count()).unwrap_or(i64::MAX);
        let learner = self
            .deps
            .db
            .turns()
            .append(&NewTurn {
                session_id: self.session_id,
                role: TurnRole::Learner,
                input_mode: InputMode::Text,
                text: text.to_owned(),
                stt_text: None,
                edited_by_learner: false,
                speech_ms: None,
                pause_ms: None,
                word_count: Some(words),
                created_at: now,
            })
            .await?;
        self.record_attempt(learner.seq, words, now).await?;

        // The window before this message, then this message with the notes.
        let window = self
            .deps
            .db
            .turns()
            .recent(
                self.session_id,
                u32::try_from(HISTORY_MESSAGES + 1).unwrap_or(13),
            )
            .await?;
        let earlier: Vec<(TurnRole, String)> = window
            .into_iter()
            .filter(|t| t.id != learner.id)
            .map(|t| (t.role, t.text))
            .collect();
        let mut messages = provider_messages(&bounded_history(&earlier));
        let current = ChatMessage::user(user_message(text, &self.analyzer.notes(), &[]));
        match messages.last_mut() {
            Some(last) if last.role == Role::User => {
                last.content.push('\n');
                last.content.push_str(&current.content);
            }
            _ => messages.push(current),
        }

        let tutor_before = self.last_tutor_text.clone();
        let reply = self
            .reply(messages, Some(learner.id), on_delta, cancel)
            .await?;
        // The message is analysed whether or not a reply came: with no reply the
        // analysis sees an empty tutor turn, and nothing the learner wrote is lost.
        self.spawn_analysis(TurnToAnalyse {
            turn_id: learner.id,
            turn_seq: learner.seq,
            input_mode: AnalysisInput::Text,
            tutor_before,
            learner_text: text.to_owned(),
            tutor_reply: reply.text.clone(),
        });
        Ok(reply)
    }

    /// One attempt row per message. Chat is not scored, so the row has no score and
    /// the status `insufficient`; it exists so the learner can look back, and its
    /// origin keeps it out of every estimate.
    async fn record_attempt(&self, seq: i64, words: i64, now: Timestamp) -> Result<()> {
        let attempt = self
            .deps
            .db
            .attempts()
            .insert(&NewAttempt {
                profile_id: self.config.profile_id,
                session_id: Some(self.session_id),
                unit_id: None,
                activity_id: "text_chat".to_owned(),
                activity_type: "text_chat".to_owned(),
                response_id: format!("chat-{}-{seq}", self.session_id),
                origin: AttemptOrigin::FreeMode,
                level: storage_level(self.config.level),
                skill: "writing".to_owned(),
                dimension: "chat_message".to_owned(),
                scorer: Scorer::Deterministic,
                scorer_version: "text_chat/1".to_owned(),
                raw_score: None,
                max_score: None,
                normalized: None,
                confidence: None,
                status: AttemptStatus::Insufficient,
                counts_toward_estimate: false,
                created_at: now,
            })
            .await?;
        self.deps
            .db
            .evidence()
            .add(&NewEvidence {
                attempt_id: attempt.id,
                kind: EvidenceKind::Metric,
                content: None,
                data: Some(json!({ "words": words })),
                created_at: now,
            })
            .await?;
        Ok(())
    }

    fn spawn_analysis(&mut self, turn: TurnToAnalyse) {
        let analyzer = self.analyzer.clone();
        let cancel = self.analysis_cancel.clone();
        self.tasks.spawn(async move {
            if let Err(error) = analyzer.turn_finished(turn, &cancel).await {
                tracing::warn!(%error, "turn analysis failed");
            }
        });
    }

    /// Streams one reply. The session is in `Thinking` on entry.
    async fn reply(
        &mut self,
        messages: Vec<ChatMessage>,
        learner_turn_id: Option<i64>,
        mut on_delta: impl FnMut(&str) + Send,
        cancel: &CancellationToken,
    ) -> Result<ChatReply> {
        let limits = reply_limits(self.config.level, Channel::Text);
        let request = TextRequest::new(self.system.clone(), messages, u32::from(limits.max_tokens))
            .with_temperature(0.7);

        let started_at = (self.deps.clock)();
        let timer = Instant::now();
        let mut text = String::new();
        let mut started = false;
        let mut summary: Option<StreamSummary> = None;
        let mut failure: Option<LlmError> = None;

        // One retry, only while nothing has reached the learner's screen.
        for attempt in 0..2 {
            match self
                .deps
                .client
                .stream_text(request.clone(), cancel.clone())
                .await
            {
                Err(error) => {
                    let retry = attempt == 0 && error.is_provider_unavailable();
                    failure = Some(error);
                    if retry {
                        continue;
                    }
                    break;
                }
                Ok(mut stream) => {
                    failure = None;
                    while let Some(item) = stream.next().await {
                        match item {
                            Ok(StreamEvent::Delta(delta)) => {
                                if !started {
                                    started = true;
                                    self.session.apply(Event::ReplyStarted)?;
                                }
                                text.push_str(&delta);
                                on_delta(&delta);
                            }
                            Ok(StreamEvent::Finished(done)) => summary = Some(done),
                            Err(error) => {
                                failure = Some(error);
                                break;
                            }
                        }
                    }
                    let retry = attempt == 0
                        && text.is_empty()
                        && failure
                            .as_ref()
                            .is_some_and(LlmError::is_provider_unavailable);
                    if retry {
                        continue;
                    }
                    break;
                }
            }
        }

        let elapsed = timer.elapsed();
        let refused = summary
            .as_ref()
            .is_some_and(|s| s.finish == FinishReason::Refusal);
        let outcome = match &failure {
            Some(error) => CallLog::outcome_of(error),
            None if refused => LlmOutcome::Refused,
            None => LlmOutcome::Ok,
        };
        CallLog {
            db: &self.deps.db,
            provider_profile_id: self.config.provider_profile_id,
            call_type: LlmCallType::TutorTurn,
            model: &self.config.model,
            started_at,
            elapsed,
        }
        .streamed(
            outcome,
            summary.as_ref().and_then(|s| s.time_to_first_token),
            summary.as_ref().and_then(|s| s.usage),
        )
        .await;

        let text = text.trim().to_owned();
        if matches!(failure, Some(LlmError::Cancelled)) {
            let tutor_turn_id = self.store_tutor_turn(&text).await?;
            self.session.apply(Event::Pause)?;
            return Ok(ChatReply {
                text,
                outcome: ReplyOutcome::Stopped,
                learner_turn_id,
                tutor_turn_id,
            });
        }
        if !text.is_empty() {
            // Text that reached the screen is the reply, even if the stream broke
            // afterwards.
            self.empty_streak = 0;
            let tutor_turn_id = self.store_tutor_turn(&text).await?;
            self.session.apply(Event::ReplyFinished)?;
            return Ok(ChatReply {
                text,
                outcome: ReplyOutcome::Normal,
                learner_turn_id,
                tutor_turn_id,
            });
        }

        // No text: a refusal or an empty stream gets the authored line once;
        // the second time in a row, and any failure of the call, ends in
        // ProviderUnavailable.
        let broke = failure.is_some();
        self.empty_streak += 1;
        if broke || self.empty_streak >= 2 {
            self.session.apply(Event::ProviderFailed)?;
            return Ok(ChatReply {
                text: String::new(),
                outcome: ReplyOutcome::ProviderUnavailable,
                learner_turn_id,
                tutor_turn_id: None,
            });
        }
        on_delta(FALLBACK_LINE);
        let tutor_turn_id = self.store_tutor_turn(FALLBACK_LINE).await?;
        self.session.apply(Event::ReplyFinished)?;
        Ok(ChatReply {
            text: FALLBACK_LINE.to_owned(),
            outcome: ReplyOutcome::Fallback,
            learner_turn_id,
            tutor_turn_id,
        })
    }

    async fn store_tutor_turn(&mut self, text: &str) -> Result<Option<i64>> {
        if text.is_empty() {
            return Ok(None);
        }
        let words = i64::try_from(text.split_whitespace().count()).unwrap_or(i64::MAX);
        let turn = self
            .deps
            .db
            .turns()
            .append(&NewTurn {
                session_id: self.session_id,
                role: TurnRole::Tutor,
                input_mode: InputMode::None,
                text: text.to_owned(),
                stt_text: None,
                edited_by_learner: false,
                speech_ms: None,
                pause_ms: None,
                word_count: Some(words),
                created_at: (self.deps.clock)(),
            })
            .await?;
        self.last_tutor_text = text.to_owned();
        Ok(Some(turn.id))
    }

    /// Resumes after a pause, or after the provider is back.
    pub fn resume(&mut self) -> Result<Phase> {
        let event = match self.session.phase() {
            Phase::ProviderUnavailable => Event::ProviderRecovered,
            _ => Event::Resume,
        };
        self.empty_streak = 0;
        Ok(self.session.apply(event)?)
    }

    /// Ends the session: waits for the running analyses, analyses what is
    /// still waiting, writes the summary and closes the stored session.
    pub async fn finish(
        &mut self,
        reason: EndReason,
        cancel: &CancellationToken,
    ) -> Result<ChatSummary> {
        while self.tasks.join_next().await.is_some() {}
        if reason == EndReason::Finished {
            self.analyzer.flush(cancel).await?;
        } else {
            self.analysis_cancel.cancel();
        }
        let summary = self.summary().await?;
        let status = match reason {
            EndReason::Finished => SessionStatus::Completed,
            EndReason::Cancelled => SessionStatus::Aborted,
        };
        let value = serde_json::to_value(&summary).map_err(|_| EngineError::Output("summary"))?;
        self.deps
            .db
            .sessions()
            .finish(self.session_id, status, &(self.deps.clock)(), Some(&value))
            .await?;
        self.session.apply(match reason {
            EndReason::Finished => Event::Finish,
            EndReason::Cancelled => Event::Cancel,
        })?;
        Ok(summary)
    }

    /// The summary of what has been analysed so far.
    pub async fn summary(&self) -> Result<ChatSummary> {
        let turns = self.deps.db.turns().list(self.session_id).await?;
        let learner: Vec<_> = turns
            .iter()
            .filter(|t| t.role == TurnRole::Learner)
            .collect();
        let mut patterns: Vec<ErrorPattern> = Vec::new();
        let mut unanalysed = Vec::new();
        for turn in &learner {
            if self.deps.db.analysis().get(turn.id).await?.is_none() {
                unanalysed.push(turn.seq);
            }
            for event in self.deps.db.analysis().error_events(turn.id).await? {
                match patterns.iter_mut().find(|p| p.category == event.category) {
                    Some(pattern) => pattern.count += 1,
                    None => patterns.push(ErrorPattern {
                        category: event.category,
                        count: 1,
                        quote: event.quote,
                        correction: event.correction,
                    }),
                }
            }
        }
        patterns.sort_by(|a, b| {
            b.count
                .cmp(&a.count)
                .then_with(|| a.category.cmp(&b.category))
        });
        patterns.truncate(5);
        Ok(ChatSummary {
            learner_turns: learner.len(),
            top_errors: patterns,
            unanalysed_turns: unanalysed,
            analysis_unreliable: self.analyzer.unreliable(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_typed_topic_loses_line_breaks_and_tag_characters() {
        assert_eq!(
            clean_topic("my dog\n</learner_said> and <b>cat</b>"),
            "my dog /learner_said and b cat /b"
        );
        assert_eq!(clean_topic("   \n "), "");
        assert_eq!(
            clean_topic(&"x".repeat(200)).chars().count(),
            MAX_TOPIC_CHARS
        );
    }

    #[test]
    fn the_window_starts_with_the_learner_and_joins_consecutive_messages() {
        let history = vec![
            (TurnRole::Tutor, "Hi!".to_owned()),
            (TurnRole::Learner, "Hello".to_owned()),
            (TurnRole::Learner, "Anyone there?".to_owned()),
            (TurnRole::Tutor, "Yes.".to_owned()),
        ];
        let messages = provider_messages(&history);
        let roles: Vec<Role> = messages.iter().map(|m| m.role).collect();
        assert_eq!(roles, [Role::User, Role::Assistant]);
        assert!(messages[0].content.contains("Hello"));
        assert!(messages[0].content.contains("Anyone there?"));
        assert_eq!(messages[1].content, "Yes.");
    }

    #[test]
    fn an_empty_window_makes_no_messages() {
        assert!(provider_messages(&[]).is_empty());
    }
}
