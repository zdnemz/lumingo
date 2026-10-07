//! Text chat (S4-11): the conversation engine on the text channel (PRD FR-C6,
//! FR-C7). One tutor, by typing, with no microphone and no speech model. The
//! scenario comes from the topic bank or from a topic the learner typed; the
//! prompt is the T1 contract on the text channel, which `prompt.rs` already
//! builds.
//!
//! The chat is practice only: every learner message is stored as an attempt
//! with origin `free_mode`, status `insufficient` and no score, so it never
//! counts toward a level estimate (ASSESSMENT_SPEC section 11). The caller
//! stores the turns and the attempt rows; this module owns the conversation
//! state, the bounded history, the fallback rule, and the summary data.
//!
//! Analysis runs exactly as it does for a voice turn: the caller builds
//! `AnalysisInput::free` for a learner turn, runs [`crate::run_analysis`], and
//! feeds the filtered outcome back with [`Chat::apply_analysis`] so the notes
//! reach the next turn.

use curriculum::Level;
use llm_client::{Message, Role, TextRequest};
use serde::Serialize;
use tokio_util::sync::CancellationToken;

use crate::analysis::{ErrorFinding, FilteredAnalysis, NotesForNextTurn};
use crate::event::UiEvent;
use crate::llm::LlmClient;
use crate::prompt::{
    FeedbackMode, T1_TEMPERATURE, TutorContext, bounded_history, opening_request, reply_limits,
    system_prompt, user_message,
};
use crate::session::{Channel, Event, Session, SessionKind, TransitionError};
use crate::topics::ConversationTopic;
use crate::turn::{ReplyOutcome, ReplyReport, run_reply};

/// Longest typed topic, in characters. It goes into the system prompt, so it is
/// kept to a title.
pub const MAX_TOPIC_CHARS: usize = 80;

/// The `scorer_version` a chat message's attempt row carries. The row exists so
/// the learner can look back; `origin = free_mode` and
/// `counts_toward_estimate = false` keep it out of every estimate.
pub const CHAT_ATTEMPT_SCORER_VERSION: &str = "text_chat/1";

/// Where the conversation's scenario comes from (PRD FR-C7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatTopic {
    /// One entry of the topic bank for the learner's level.
    Bank(ConversationTopic),
    /// A topic the learner typed; it is cleaned and capped by [`clean_topic`].
    Typed(String),
}

/// What one chat needs to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatConfig {
    pub model: String,
    /// The level the tutor speaks at: the learner's pick or their current
    /// estimate. Never a value a model produced.
    pub level: Level,
    pub mode: FeedbackMode,
    /// The first language written in English ("Indonesian").
    pub first_language: String,
    pub topic: ChatTopic,
}

/// Why a chat could not start or continue.
#[derive(Debug, thiserror::Error)]
pub enum ChatError {
    #[error("the typed topic is empty")]
    EmptyTopic,
    #[error("the message is empty")]
    EmptyMessage,
    #[error(transparent)]
    Transition(#[from] TransitionError),
}

/// One chat turn: the tutor's reply and the stored sequence numbers the caller
/// should give the turns. The opening turn has no learner message.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatTurn {
    pub report: ReplyReport,
    /// The seq of the learner's message in the stored session; `None` for the
    /// tutor's opening message.
    pub learner_seq: Option<i64>,
    /// The seq of the tutor's message.
    pub tutor_seq: i64,
}

/// The attempt row data for one stored learner message. The caller fills a
/// `storage::NewAttempt` from it with origin `free_mode`, status
/// `insufficient`, no scores, and `counts_toward_estimate = false`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatAttempt {
    pub response_id: String,
    pub activity_id: String,
    pub activity_type: String,
    pub level: Level,
    pub dimension: String,
    pub scorer_version: String,
    /// The message's word count, kept as metric evidence.
    pub words: i64,
}

/// A text chat in progress: the session state machine on the text channel, the
/// bounded history, the notes from the latest analysis, and the fallback rule.
#[derive(Debug, Clone)]
pub struct Chat {
    model: String,
    context: TutorContext,
    /// The topic-bank entry's id, when the scenario came from the bank; `None`
    /// for a typed topic.
    topic_id: Option<String>,
    session: Session,
    history: Vec<Message>,
    notes: NotesForNextTurn,
    empty_streak: u32,
    next_seq: i64,
}

impl Chat {
    /// Starts a chat. The topic decides the scenario: a bank entry fills the
    /// T1 fields from the catalog, a typed topic uses the contract's fixed
    /// wording for a free topic.
    pub fn start(config: ChatConfig) -> Result<Self, ChatError> {
        let context = match &config.topic {
            ChatTopic::Bank(topic) => TutorContext::from_bank_topic(
                topic,
                config.level,
                Channel::Text,
                config.mode,
                &config.first_language,
            ),
            ChatTopic::Typed(text) => {
                let title = clean_topic(text);
                if title.is_empty() {
                    return Err(ChatError::EmptyTopic);
                }
                TutorContext::from_topic(
                    &title,
                    config.level,
                    Channel::Text,
                    config.mode,
                    &config.first_language,
                )
            }
        };
        let topic_id = match &config.topic {
            ChatTopic::Bank(topic) => Some(topic.id.clone()),
            ChatTopic::Typed(_) => None,
        };
        Ok(Self {
            model: config.model,
            context,
            topic_id,
            session: Session::new(SessionKind::TextChat, Channel::Text),
            history: Vec::new(),
            notes: NotesForNextTurn::new(),
            empty_streak: 0,
            next_seq: 1,
        })
    }

    /// The scenario the session runs on.
    pub fn context(&self) -> &TutorContext {
        &self.context
    }

    /// The topic-bank entry's id, for the stored session's `activity_id`.
    pub fn topic_id(&self) -> Option<&str> {
        self.topic_id.as_deref()
    }

    /// The session state machine, for the caller that forwards phases.
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// The tutor's opening message (T1's trigger: session start when the tutor
    /// speaks first). The caller stores it at `tutor_seq`.
    pub async fn open(
        &mut self,
        client: &dyn LlmClient,
        cancel: &CancellationToken,
        emit: &mut dyn FnMut(UiEvent),
    ) -> Result<ChatTurn, ChatError> {
        self.session.apply(Event::OpeningTurn)?;
        let request = opening_request(&self.model, &self.context, &self.request_history(None));
        let report = run_reply(
            &mut self.session,
            client,
            request,
            cancel,
            self.empty_streak == 0,
            emit,
        )
        .await?;
        let tutor_seq = self.next_seq;
        self.next_seq += 1;
        self.after_reply(&report);
        Ok(ChatTurn {
            report,
            learner_seq: None,
            tutor_seq,
        })
    }

    /// One learner message: stores nothing itself, streams the tutor's reply
    /// through `emit`, and returns the report with the seq numbers to store.
    /// The caller appends the learner turn at `learner_seq` and the tutor turn
    /// at `tutor_seq`, stores the message's `free_mode` attempt, and runs the
    /// analysis.
    pub async fn say(
        &mut self,
        client: &dyn LlmClient,
        text: &str,
        cancel: &CancellationToken,
        emit: &mut dyn FnMut(UiEvent),
    ) -> Result<ChatTurn, ChatError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(ChatError::EmptyMessage);
        }
        self.session.apply(Event::TextSent)?;
        let learner_seq = self.next_seq;
        let tutor_seq = learner_seq + 1;
        let request = self.turn_request(text);
        let report = run_reply(
            &mut self.session,
            client,
            request,
            cancel,
            self.empty_streak == 0,
            emit,
        )
        .await?;
        self.next_seq += 2;
        self.history.push(Message {
            role: Role::User,
            content: text.to_owned(),
        });
        self.after_reply(&report);
        Ok(ChatTurn {
            report,
            learner_seq: Some(learner_seq),
            tutor_seq,
        })
    }

    /// The attempt row data for one stored learner message (PRD FR-C6: chat is
    /// practice). The caller maps it to `storage::NewAttempt`: origin
    /// `free_mode`, status `insufficient`, no score, `counts_toward_estimate =
    /// false`, so it never enters a level estimate (ASSESSMENT_SPEC section 11)
    /// but the learner can look it back up.
    pub fn attempt_for(&self, session_id: i64, seq: i64, words: i64) -> ChatAttempt {
        ChatAttempt {
            response_id: format!("chat-{session_id}-{seq}"),
            activity_id: "text_chat".to_owned(),
            activity_type: "text_chat".to_owned(),
            level: self.context.level,
            dimension: "chat_message".to_owned(),
            scorer_version: CHAT_ATTEMPT_SCORER_VERSION.to_owned(),
            words,
        }
    }

    /// The newest analysis's notes, ready for the next turn (T1's
    /// `note_for_next_turn` values).
    pub fn apply_analysis(&mut self, filtered: &FilteredAnalysis) {
        self.notes.update(filtered);
    }

    /// The provider came back: resume where the chat left off. A session that
    /// is not `ProviderUnavailable` refuses the event and changes nothing.
    pub fn recover(&mut self) -> Result<(), ChatError> {
        self.session.apply(Event::ProviderRecovered)?;
        self.empty_streak = 0;
        Ok(())
    }

    fn after_reply(&mut self, report: &ReplyReport) {
        if !report.text.is_empty() {
            self.history.push(Message {
                role: Role::Assistant,
                content: report.text.clone(),
            });
        }
        // A second empty or refused answer in a row is a provider problem, not
        // a silent tutor (T1's output handling).
        self.empty_streak = if report.outcome == ReplyOutcome::Fallback {
            self.empty_streak + 1
        } else {
            0
        };
    }

    /// The provider messages for the next call: the bounded window (T1 section
    /// 8) with consecutive roles joined, learner text wrapped in the turn
    /// message, leading tutor messages dropped, and the current learner message
    /// (when there is one) appended with the newest notes.
    fn request_history(&self, current: Option<(&str, &[String])>) -> Vec<Message> {
        let mut out = provider_history(bounded_history(&self.history).as_slice());
        if let Some((text, notes)) = current {
            let message = Message {
                role: Role::User,
                content: user_message(text, notes, &[]),
            };
            match out.last_mut() {
                Some(last) if last.role == Role::User => {
                    last.content.push('\n');
                    last.content.push_str(&message.content);
                }
                _ => out.push(message),
            }
        }
        out
    }

    /// The request for one learner turn on the text channel.
    fn turn_request(&self, text: &str) -> TextRequest {
        let limits = reply_limits(self.context.level, Channel::Text);
        TextRequest {
            model: self.model.clone(),
            system: Some(system_prompt(&self.context)),
            messages: self.request_history(Some((text, self.notes.notes()))),
            max_tokens: u32::from(limits.max_tokens),
            temperature: Some(T1_TEMPERATURE),
        }
    }
}

/// A typed topic reduced to one safe line for the system prompt: control
/// characters, line breaks and tag brackets become spaces, runs of spaces
/// collapse, and the result is cut to [`MAX_TOPIC_CHARS`] without a trailing
/// space.
pub fn clean_topic(topic: &str) -> String {
    let flat: String = topic
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>') {
                ' '
            } else {
                c
            }
        })
        .collect();
    let collapsed = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed
        .chars()
        .take(MAX_TOPIC_CHARS)
        .collect::<String>()
        .trim_end()
        .to_owned()
}

/// Turns the in-memory window into provider messages: learner text is wrapped
/// in the turn message, consecutive messages of one role are joined (a learner
/// message whose reply failed would otherwise leave two learner messages in a
/// row), and leading assistant messages are dropped, because some providers
/// reject a conversation that opens with the assistant.
fn provider_history(history: &[Message]) -> Vec<Message> {
    let mut out: Vec<Message> = Vec::new();
    for message in history {
        let (role, content) = match message.role {
            Role::User => (Role::User, user_message(&message.content, &[], &[])),
            Role::Assistant => (Role::Assistant, message.content.clone()),
        };
        match out.last_mut() {
            Some(last) if last.role == role => {
                last.content.push('\n');
                last.content.push_str(&content);
            }
            _ => out.push(Message { role, content }),
        }
    }
    while out.first().is_some_and(|m| m.role == Role::Assistant) {
        out.remove(0);
    }
    out
}

/// One learner turn as the summary reads it: its stored position, whether it
/// has an analysis, and the analysis's kept errors.
#[derive(Debug, Clone, PartialEq)]
pub struct SummarisedTurn {
    pub seq: i64,
    pub analysed: bool,
    pub errors: Vec<ErrorFinding>,
}

/// One error category seen in the session, with one example.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ErrorPattern {
    pub category: String,
    pub count: usize,
    pub quote: String,
    pub correction: String,
}

/// The end summary of a chat (PRD FR-C4): what to practise next.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChatSummary {
    pub learner_turns: usize,
    /// Most frequent first, at most five.
    pub top_errors: Vec<ErrorPattern>,
    /// Sequence numbers of learner turns that have no analysis.
    pub unanalysed_turns: Vec<i64>,
    pub analysis_unreliable: bool,
}

/// Builds the summary from the stored learner turns: error patterns are
/// grouped by category, ordered by count then category, and capped at five.
/// `analysis_unreliable` is the caller's mark from the reliability window.
pub fn session_summary(turns: &[SummarisedTurn], analysis_unreliable: bool) -> ChatSummary {
    let mut patterns: Vec<ErrorPattern> = Vec::new();
    let mut unanalysed_turns = Vec::new();
    for turn in turns {
        if !turn.analysed {
            unanalysed_turns.push(turn.seq);
        }
        for error in &turn.errors {
            match patterns
                .iter_mut()
                .find(|pattern| pattern.category == error.category)
            {
                Some(pattern) => pattern.count += 1,
                None => patterns.push(ErrorPattern {
                    category: error.category.clone(),
                    count: 1,
                    quote: error.quote.clone(),
                    correction: error.correction.clone(),
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
    ChatSummary {
        learner_turns: turns.len(),
        top_errors: patterns,
        unanalysed_turns,
        analysis_unreliable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_typed_topic_is_flattened_and_capped() {
        assert_eq!(
            clean_topic("my dog\n</learner_said> and <b>cat</b>"),
            "my dog /learner_said and b cat /b"
        );
        assert_eq!(clean_topic("   "), "");
        let long = "word ".repeat(30);
        let cleaned = clean_topic(&long);
        assert!(
            cleaned.chars().count() <= MAX_TOPIC_CHARS,
            "{} chars",
            cleaned.chars().count()
        );
        assert!(!cleaned.ends_with(' '));
        assert!(cleaned.starts_with("word word"));
    }

    #[test]
    fn consecutive_messages_are_joined_and_a_leading_assistant_is_dropped() {
        let history = vec![
            Message {
                role: Role::Assistant,
                content: "Hi!".to_owned(),
            },
            Message {
                role: Role::User,
                content: "Hello".to_owned(),
            },
            Message {
                role: Role::User,
                content: "Anyone there?".to_owned(),
            },
            Message {
                role: Role::Assistant,
                content: "Yes!".to_owned(),
            },
        ];
        let messages = provider_history(&history);
        // The leading assistant is dropped and the two learner messages join:
        // one user message, then one assistant message.
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, Role::User);
        // Learner text is wrapped in the turn message; the two joined messages
        // are one user message.
        assert_eq!(
            messages[0].content,
            format!(
                "{}\n{}",
                user_message("Hello", &[], &[]),
                user_message("Anyone there?", &[], &[])
            )
        );
        assert!(
            messages[0]
                .content
                .contains("<learner_said>\nHello\n</learner_said>")
        );
        assert_eq!(messages[1].role, Role::Assistant);
        assert_eq!(messages[1].content, "Yes!");
        assert!(provider_history(&[]).is_empty());
        // A history of only tutor messages has nothing to send.
        let only_tutor = vec![Message {
            role: Role::Assistant,
            content: "Hi!".to_owned(),
        }];
        assert!(provider_history(&only_tutor).is_empty());
    }

    fn finding(category: &str, quote: &str) -> ErrorFinding {
        ErrorFinding {
            category: category.to_owned(),
            quote: quote.to_owned(),
            correction: "x".to_owned(),
            severity: "major".to_owned(),
            addressed_in_reply: false,
        }
    }

    #[test]
    fn the_summary_groups_errors_by_category_and_lists_unanalysed_turns() {
        let turns = vec![
            SummarisedTurn {
                seq: 2,
                analysed: true,
                errors: vec![
                    finding("verb_tense", "go"),
                    finding("word_choice", "brother"),
                ],
            },
            SummarisedTurn {
                seq: 4,
                analysed: true,
                errors: vec![finding("verb_tense", "watch")],
            },
            SummarisedTurn {
                seq: 6,
                analysed: false,
                errors: Vec::new(),
            },
        ];
        let summary = session_summary(&turns, false);
        assert_eq!(summary.learner_turns, 3);
        assert_eq!(summary.unanalysed_turns, [6]);
        assert!(!summary.analysis_unreliable);
        assert_eq!(summary.top_errors.len(), 2);
        assert_eq!(summary.top_errors[0].category, "verb_tense");
        assert_eq!(summary.top_errors[0].count, 2);
        assert_eq!(summary.top_errors[0].quote, "go");
        assert_eq!(summary.top_errors[1].category, "word_choice");
        assert_eq!(summary.top_errors[1].count, 1);
    }

    #[test]
    fn the_summary_keeps_at_most_five_patterns_ordered_by_count_then_category() {
        let mut turns = Vec::new();
        for (i, category) in ["a", "b", "c", "d", "e", "f"].iter().enumerate() {
            turns.push(SummarisedTurn {
                seq: i as i64 + 1,
                analysed: true,
                errors: vec![finding(category, "q")],
            });
        }
        // "f" and "a" both have one hit; the cap keeps five, ordered by category.
        let summary = session_summary(&turns, true);
        assert_eq!(summary.top_errors.len(), 5);
        let categories: Vec<&str> = summary
            .top_errors
            .iter()
            .map(|p| p.category.as_str())
            .collect();
        assert_eq!(categories, ["a", "b", "c", "d", "e"]);
        assert!(summary.analysis_unreliable);
    }

    #[test]
    fn an_empty_chat_summarises_to_nothing() {
        let summary = session_summary(&[], false);
        assert_eq!(summary.learner_turns, 0);
        assert!(summary.top_errors.is_empty());
        assert!(summary.unanalysed_turns.is_empty());
    }

    #[test]
    fn a_bank_topic_fills_the_context_from_the_catalog_entry() {
        let topic = ConversationTopic {
            id: "cafe".to_owned(),
            title: curriculum::Localized {
                en: "At a cafe".to_owned(),
                id: Some("Di kafe".to_owned()),
            },
            scenario: curriculum::Localized {
                en: "You order a drink and pay.".to_owned(),
                id: None,
            },
            tutor_role: "a friendly waiter".to_owned(),
            learner_role: "a customer".to_owned(),
            goals: vec!["Order a drink".to_owned()],
        };
        let chat = Chat::start(ChatConfig {
            model: "m".to_owned(),
            level: Level::A1,
            mode: FeedbackMode::Fluency,
            first_language: "Indonesian".to_owned(),
            topic: ChatTopic::Bank(topic),
        })
        .unwrap();
        assert_eq!(chat.context().scenario, "You order a drink and pay.");
        assert_eq!(chat.context().tutor_role, "a friendly waiter");
        assert_eq!(chat.context().goals, ["Order a drink"]);
        assert_eq!(chat.session().kind(), SessionKind::TextChat);
        assert_eq!(chat.session().channel(), Channel::Text);
    }

    #[test]
    fn a_typed_topic_uses_the_contract_wording_and_an_empty_one_is_refused() {
        let chat = Chat::start(ChatConfig {
            model: "m".to_owned(),
            level: Level::B1,
            mode: FeedbackMode::Fluency,
            first_language: "Indonesian".to_owned(),
            topic: ChatTopic::Typed("  Travel\nplans  ".to_owned()),
        })
        .unwrap();
        assert_eq!(
            chat.context().scenario,
            "an open conversation about this topic"
        );
        assert_eq!(chat.context().tutor_role, "a friendly conversation partner");

        let error = Chat::start(ChatConfig {
            model: "m".to_owned(),
            level: Level::B1,
            mode: FeedbackMode::Fluency,
            first_language: "Indonesian".to_owned(),
            topic: ChatTopic::Typed(" \n ".to_owned()),
        })
        .unwrap_err();
        assert!(matches!(error, ChatError::EmptyTopic));
    }
}
