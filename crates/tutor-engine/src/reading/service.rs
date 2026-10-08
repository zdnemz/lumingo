//! The graded reading session: generate a text, check it, regenerate once,
//! fall back to authored sets, store what was generated with the session, and
//! score the answers.

use std::sync::Arc;
use std::time::Instant;

use assessment_engine::{Level, Origin, score_share};
use curriculum::Unit;
use curriculum::validate::WordLevels;
use llm_client::{ChatMessage, Contract, LlmClient, LlmError, StructuredRequest};
use serde_json::json;
use storage::{
    Database, GeneratedKind, LlmCallType, NewGeneratedContent, NewSession, SessionStatus, Timestamp,
};
use tokio_util::sync::CancellationToken;

use crate::chat::clean_topic;
use crate::error::{EngineError, Result};
use crate::evidence::{DeterministicRecord, EvidenceRecorder, Subject};
use crate::support::{CallLog, Clock};
use crate::topics::ReadingTopic;

use super::check::{ReadingProblem, check_reading};
use super::prompt::{READING_PASSAGE_VERSION, ReadingSpec, USER_MESSAGE, system_prompt};
use super::{AuthoredReading, RawReading, authored_reading_sets};

/// Glossary entries kept, at most (the contract asks for 5 to 8).
const MAX_GLOSSARY: usize = 8;

/// What a reading session works with.
#[derive(Clone)]
pub struct ReadingDeps {
    pub client: Arc<dyn LlmClient>,
    pub db: Database,
    pub clock: Clock,
}

#[derive(Debug, Clone)]
pub struct ReadingConfig {
    pub profile_id: i64,
    pub provider_profile_id: Option<i64>,
    pub model: String,
    /// The level of the text, picked by the learner. Never a value a model produced.
    pub level: Level,
    /// The first language written in English ("Indonesian").
    pub first_language: String,
    pub app_version: String,
    /// Word levels for the vocabulary profile. Without a list that check is
    /// skipped and the result says so.
    pub word_levels: Option<Arc<WordLevels>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadingTopicChoice {
    Bank(ReadingTopic),
    Typed(String),
}

/// A generated text that passed the checks. It is always generated content: the
/// view labels it so and says that details may be wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedReading {
    /// The stored row, for reopening the text and for scoring answers.
    pub content_id: i64,
    pub reading: RawReading,
    /// The first text failed a check and this is the regeneration.
    pub regenerated: bool,
    /// False when no word list was available for the vocabulary profile.
    pub vocabulary_checked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadingFallbackReason {
    ProviderUnavailable,
    /// The reply was not usable twice: invalid output, or failed checks.
    Unusable,
}

/// Authored reading sets, offered in place of a generated text.
#[derive(Debug, Clone, PartialEq)]
pub struct ReadingFallback {
    pub reason: ReadingFallbackReason,
    pub sets: Vec<AuthoredReading>,
    /// What the second text failed, if it was readable at all.
    pub problems: Vec<ReadingProblem>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReadingOutcome {
    Generated(GeneratedReading),
    Fallback(ReadingFallback),
}

/// The result of one question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionResult {
    pub chosen: Option<usize>,
    pub correct_index: usize,
    pub correct: bool,
    pub explanation_en: String,
    pub explanation_l1: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReadingScore {
    pub correct: usize,
    pub total: usize,
    /// 0 to 1. Practice only.
    pub score: f64,
    pub questions: Vec<QuestionResult>,
}

/// A generated text kept with its session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredReading {
    pub content_id: i64,
    pub reading: RawReading,
    pub model: String,
    pub created_at: Timestamp,
}

pub struct ReadingSession {
    config: ReadingConfig,
    deps: ReadingDeps,
    recorder: EvidenceRecorder,
    session_id: i64,
}

struct AnswerKey {
    answer_index: usize,
    explanation_en: String,
    explanation_l1: String,
}

impl ReadingSession {
    pub async fn start(deps: ReadingDeps, config: ReadingConfig) -> Result<Self> {
        let stored = deps
            .db
            .sessions()
            .create(&NewSession {
                profile_id: config.profile_id,
                kind: storage::SessionKind::Reading,
                unit_id: None,
                activity_id: None,
                mode: None,
                provider_profile_id: config.provider_profile_id,
                app_version: config.app_version.clone(),
                started_at: (deps.clock)(),
            })
            .await?;
        Ok(Self {
            recorder: EvidenceRecorder::new(deps.db.clone(), deps.clock.clone()),
            config,
            deps,
            session_id: stored.id,
        })
    }

    pub fn session_id(&self) -> i64 {
        self.session_id
    }

    /// Generates a text for the topic and checks it. A text that fails a check,
    /// or a reply that stays invalid, is regenerated once. When that fails too,
    /// or the provider cannot be reached, the outcome is a fallback to the
    /// authored reading sets of `units` at the learner's level. Only a cancelled
    /// call and storage failures are errors.
    pub async fn generate(
        &self,
        topic: &ReadingTopicChoice,
        units: &[Unit],
        cancel: &CancellationToken,
    ) -> Result<ReadingOutcome> {
        let title = match topic {
            ReadingTopicChoice::Bank(t) => clean_topic(&t.title.en),
            ReadingTopicChoice::Typed(text) => clean_topic(text),
        };
        if title.is_empty() {
            return Err(EngineError::Refused("the topic is empty"));
        }
        let spec = ReadingSpec::for_level(self.config.level);
        let fallback = |reason, problems| {
            ReadingOutcome::Fallback(ReadingFallback {
                reason,
                sets: authored_reading_sets(units, self.config.level),
                problems,
            })
        };

        let mut problems = Vec::new();
        for attempt in 0..2 {
            match self.call(&spec, &title, cancel).await {
                Ok(mut raw) => {
                    problems = check_reading(&raw, &spec, self.config.word_levels.as_deref());
                    if !problems.is_empty() {
                        continue;
                    }
                    raw.questions.truncate(spec.question_count);
                    raw.glossary.truncate(MAX_GLOSSARY);
                    let content_id = self.store(&raw).await?;
                    return Ok(ReadingOutcome::Generated(GeneratedReading {
                        content_id,
                        reading: raw,
                        regenerated: attempt == 1,
                        vocabulary_checked: self.config.word_levels.is_some(),
                    }));
                }
                Err(EngineError::Llm(LlmError::Cancelled)) => {
                    return Err(EngineError::Llm(LlmError::Cancelled));
                }
                Err(EngineError::Llm(LlmError::InvalidOutput(_))) | Err(EngineError::Output(_)) => {
                    problems = Vec::new();
                }
                Err(EngineError::Llm(_)) => {
                    return Ok(fallback(
                        ReadingFallbackReason::ProviderUnavailable,
                        Vec::new(),
                    ));
                }
                Err(other) => return Err(other),
            }
        }
        Ok(fallback(ReadingFallbackReason::Unusable, problems))
    }

    async fn call(
        &self,
        spec: &ReadingSpec,
        topic: &str,
        cancel: &CancellationToken,
    ) -> Result<RawReading> {
        let request = StructuredRequest::new(
            Contract::ReadingPassage,
            system_prompt(spec, topic, &self.config.first_language),
            vec![ChatMessage::user(USER_MESSAGE)],
            u32::try_from(
                spec.max_words * 3 / 2 + 150 * spec.question_count + 70 * spec.glossary_count + 200,
            )
            .unwrap_or(4000),
        )
        .with_temperature(0.7);
        let started_at = (self.deps.clock)();
        let timer = Instant::now();
        let result = self.deps.client.structured(request, cancel.clone()).await;
        CallLog {
            db: &self.deps.db,
            provider_profile_id: self.config.provider_profile_id,
            call_type: LlmCallType::ReadingGen,
            model: &self.config.model,
            started_at,
            elapsed: timer.elapsed(),
        }
        .structured(&result)
        .await;
        let output = result?;
        serde_json::from_value(output.value).map_err(|_| EngineError::Output("reading_passage"))
    }

    async fn store(&self, raw: &RawReading) -> Result<i64> {
        let content =
            serde_json::to_value(raw).map_err(|_| EngineError::Output("reading_passage"))?;
        let stored = self
            .deps
            .db
            .generated_content()
            .add(&NewGeneratedContent {
                session_id: self.session_id,
                kind: GeneratedKind::ReadingPassage,
                content,
                contract_version: READING_PASSAGE_VERSION.to_owned(),
                model: self.config.model.clone(),
                created_at: (self.deps.clock)(),
            })
            .await?;
        Ok(stored.id)
    }

    /// Scores the answers to a generated text of this session. `answers[i]` is
    /// the option chosen for question `i`, or `None` when it was left open.
    pub async fn score_generated(
        &self,
        content_id: i64,
        answers: &[Option<usize>],
    ) -> Result<ReadingScore> {
        let stored = list_generated_readings(&self.deps.db, self.session_id)
            .await?
            .into_iter()
            .find(|s| s.content_id == content_id)
            .ok_or(EngineError::Refused("this text is not part of the session"))?;
        let keys = stored
            .reading
            .questions
            .iter()
            .map(|q| {
                usize::try_from(q.answer_index)
                    .map(|answer_index| AnswerKey {
                        answer_index,
                        explanation_en: q.explanation_en.clone(),
                        explanation_l1: q.explanation_l1.clone(),
                    })
                    .map_err(|_| EngineError::Output("reading_passage"))
            })
            .collect::<Result<Vec<_>>>()?;
        self.score(format!("generated-{content_id}"), None, &keys, answers)
            .await
    }

    /// Scores the answers to an authored set that was offered as the fallback.
    /// It is stored as free-mode practice like the generated texts: the learner
    /// chose it inside graded reading, and the unit's own activity is where it
    /// counts.
    pub async fn score_authored(
        &self,
        set: &AuthoredReading,
        answers: &[Option<usize>],
    ) -> Result<ReadingScore> {
        let keys: Vec<AnswerKey> = set
            .questions
            .iter()
            .map(|q| AnswerKey {
                answer_index: usize::from(q.answer_index),
                explanation_en: q.explanation.en.clone(),
                explanation_l1: q.explanation.id.clone().unwrap_or_default(),
            })
            .collect();
        self.score(
            format!("{}/{}", set.unit_id, set.activity_id),
            Some(set.unit_id.clone()),
            &keys,
            answers,
        )
        .await
    }

    async fn score(
        &self,
        activity_id: String,
        unit_id: Option<String>,
        keys: &[AnswerKey],
        answers: &[Option<usize>],
    ) -> Result<ReadingScore> {
        if keys.is_empty() || answers.len() != keys.len() {
            return Err(EngineError::Refused(
                "there must be one answer, or none, for every question",
            ));
        }
        let questions: Vec<QuestionResult> = keys
            .iter()
            .zip(answers)
            .map(|(key, chosen)| QuestionResult {
                chosen: *chosen,
                correct_index: key.answer_index,
                correct: *chosen == Some(key.answer_index),
                explanation_en: key.explanation_en.clone(),
                explanation_l1: key.explanation_l1.clone(),
            })
            .collect();
        let correct = questions.iter().filter(|q| q.correct).count();
        let total = questions.len();
        let score = score_share(correct, total);

        let subject = Subject::new(
            self.config.profile_id,
            Some(self.session_id),
            unit_id,
            activity_id.clone(),
            "graded_reading",
            self.config.level,
            "reading",
            Origin::FreeMode,
            self.recorder.new_response_id(&activity_id),
        );
        self.recorder
            .record_deterministic(
                &subject,
                &DeterministicRecord {
                    normalized: score,
                    raw: correct as f64,
                    max: total as f64,
                    algorithm: "graded_reading/1",
                    response_text: None,
                    details: json!({ "correct": correct, "total": total }),
                    notes: Vec::new(),
                },
            )
            .await?;
        Ok(ReadingScore {
            correct,
            total,
            score,
            questions,
        })
    }

    /// Ends the session. Generated texts stay with it until it is deleted.
    pub async fn finish(&self, aborted: bool) -> Result<()> {
        let texts = list_generated_readings(&self.deps.db, self.session_id)
            .await?
            .len();
        let answered = self
            .deps
            .db
            .attempts()
            .for_session(self.session_id)
            .await?
            .len();
        let status = if aborted {
            SessionStatus::Aborted
        } else {
            SessionStatus::Completed
        };
        self.deps
            .db
            .sessions()
            .finish(
                self.session_id,
                status,
                &(self.deps.clock)(),
                Some(&json!({ "generated_texts": texts, "answer_sets": answered })),
            )
            .await?;
        Ok(())
    }
}

/// The generated texts of a session, oldest first, so a text can be reopened
/// until its session is deleted.
pub async fn list_generated_readings(db: &Database, session_id: i64) -> Result<Vec<StoredReading>> {
    let rows = db.generated_content().for_session(session_id).await?;
    Ok(rows
        .into_iter()
        .filter(|r| r.kind == GeneratedKind::ReadingPassage)
        .filter_map(|r| {
            serde_json::from_value::<RawReading>(r.content)
                .ok()
                .map(|reading| StoredReading {
                    content_id: r.id,
                    reading,
                    model: r.model,
                    created_at: r.created_at,
                })
        })
        .collect())
}
