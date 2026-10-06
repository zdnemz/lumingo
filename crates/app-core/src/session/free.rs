//! The free modes: the writing workshop and graded reading. Both are practice
//! only. Their attempts are stored with the origin `free_mode`, so they never
//! reach an estimate, and a generated text is always labelled generated.

use std::sync::Arc;

use async_trait::async_trait;
use curriculum::Unit;
use storage::SessionStatus;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tutor_engine::{
    DraftSubmission, GeneratedReading, ReadingConfig, ReadingDeps, ReadingFallbackReason,
    ReadingOutcome, ReadingSession, ReadingTopicChoice, TaskFamily, Workshop, WorkshopConfig,
    WorkshopEnv, WorkshopTask, authored_reading_sets, clean_topic,
};

use super::convert::{draft_feedback, reading_score};
use super::emit::Emitter;
use super::env::{Env, assessment_level, engine_error, unit_level};
use super::manager::{Ended, Run, STOP_WAIT, Shared, lock};
use crate::api::{
    ActiveSessionView, AnswerReadingRequest, AuthoredSetView, DraftAccepted, FeedbackView,
    GlossEntryView, ReadingAnswered, ReadingFallbackReasonView, ReadingOutcomeView,
    ReadingQuestionView, ReadingTarget, ReadingTextView, SessionChannel, SessionKind, SessionLife,
    StartSessionRequest, TopicChoice, TurnPhase,
};
use crate::error::{CoreError, CoreResult};

/// The longest draft, in characters. It is the same limit the activity runtime
/// puts on a text response.
const MAX_DRAFT_CHARS: usize = tutor_engine::MAX_TEXT_CHARS;

fn level_of(request: &StartSessionRequest) -> assessment_engine::Level {
    request
        .level
        .map_or(assessment_engine::Level::A1, assessment_level)
}

fn view_of(kind: SessionKind, id: i64, channel: SessionChannel) -> ActiveSessionView {
    ActiveSessionView {
        id,
        kind,
        unit_id: None,
        channel,
        life: SessionLife::Active,
        turn_state: Some(TurnPhase::Waiting),
        fault: None,
        turns_completed: 0,
        turn: 0,
        recent: Vec::new(),
        pending_reply: None,
        speaking: false,
        activity_id: None,
    }
}

// ---- the writing workshop ------------------------------------------------------

pub(crate) struct WritingRun {
    emit: Emitter,
    workshop: Arc<Workshop>,
    analyses: std::sync::Mutex<JoinSet<()>>,
    drafts: std::sync::atomic::AtomicU64,
    token: CancellationToken,
}

/// The task of a writing session: a prompt of the bank, or the learner's own.
fn writing_task(
    shared: &Shared,
    request: &StartSessionRequest,
    level: assessment_engine::Level,
) -> CoreResult<(WorkshopTask, Option<String>)> {
    match &request.topic {
        None => Err(CoreError::InvalidInput(
            "a writing session needs a prompt or a text to work on".to_owned(),
        )),
        Some(TopicChoice::Typed { text }) => {
            let prompt = clean_topic(text);
            if prompt.is_empty() {
                return Err(CoreError::InvalidInput("the prompt is empty".to_owned()));
            }
            Ok((
                WorkshopTask {
                    prompt,
                    content_points: Vec::new(),
                    min_words: None,
                },
                None,
            ))
        }
        Some(TopicChoice::Bank { id }) => {
            let bank = shared.catalogs.topics.as_ref().ok_or_else(|| {
                CoreError::unavailable(
                    None,
                    "the topic bank is not installed (catalogs/topics.json), so only your own prompt works",
                )
            })?;
            let prompt = bank.writing_prompt(level, id).ok_or(CoreError::NotFound {
                what: "writing prompt",
            })?;
            Ok((
                WorkshopTask {
                    prompt: prompt.prompt.en.clone(),
                    content_points: prompt.content_points.clone(),
                    min_words: None,
                },
                Some(prompt.id.clone()),
            ))
        }
    }
}

pub(crate) async fn start_writing(
    shared: &Arc<Shared>,
    env: Env,
    request: &StartSessionRequest,
) -> CoreResult<Arc<WritingRun>> {
    let level = level_of(request);
    let (task, prompt_id) = writing_task(shared, request, level)?;
    let rubric = shared
        .catalogs
        .rubrics
        .for_family(level, TaskFamily::WrittenProduction)
        .map(|r| r.rubric.clone());
    let workshop = Workshop::start(
        WorkshopEnv {
            client: env.llm.clone(),
            db: env.db.clone(),
            clock: env.clock.clone(),
            model: env.model.clone(),
            provider_profile_id: env.provider_profile_id,
            grammar: None,
            word_levels: shared.catalogs.word_levels.clone(),
            provider_qualified: env.provider_qualified,
        },
        WorkshopConfig {
            profile_id: env.profile_id,
            level,
            first_language: env.first_language.clone(),
            app_version: env.app_version.clone(),
            prompt_id,
            rubric,
            task,
        },
    )
    .await
    .map_err(engine_error)?;
    let emit = Emitter::new(
        Arc::clone(&shared.bus),
        view_of(
            SessionKind::Writing,
            workshop.session_id(),
            SessionChannel::Text,
        ),
    );
    Ok(Arc::new(WritingRun {
        emit,
        workshop: Arc::new(workshop),
        analyses: std::sync::Mutex::new(JoinSet::new()),
        drafts: std::sync::atomic::AtomicU64::new(0),
        token: shared.shutdown.child_token(),
    }))
}

#[async_trait]
impl Run for WritingRun {
    fn emitter(&self) -> &Emitter {
        &self.emit
    }

    async fn submit_draft(&self, text: String) -> CoreResult<DraftAccepted> {
        let text = text.trim().to_owned();
        if text.is_empty() {
            return Err(CoreError::InvalidInput("the draft is empty".to_owned()));
        }
        if text.chars().count() > MAX_DRAFT_CHARS {
            return Err(CoreError::InvalidInput(format!(
                "the draft is longer than {MAX_DRAFT_CHARS} characters"
            )));
        }
        // Layer one: stored and checked by rules at once, with no provider.
        let submission = self
            .workshop
            .submit_draft(&text)
            .await
            .map_err(engine_error)?;
        let draft = self
            .drafts
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1;
        self.emit.learner_line(
            draft,
            &submission.text,
            SessionChannel::Text,
            Some(submission.turn_seq),
            false,
        );
        let accepted = accepted_of(self.emit.id(), &submission);
        // Layers two and three can take a minute: they run on their own task and
        // arrive as `FeedbackReady`.
        let workshop = Arc::clone(&self.workshop);
        let emit = self.emit.clone();
        let token = self.token.child_token();
        lock(&self.analyses).spawn(async move {
            match workshop.analyse_draft(&submission, &token).await {
                Ok(feedback) => {
                    if let Some(record) = &feedback.analysis {
                        emit.analysis(super::convert::analysis(record));
                    }
                    emit.feedback(FeedbackView::Draft {
                        feedback: draft_feedback(&feedback),
                    });
                }
                Err(error) => {
                    let error = engine_error(error);
                    let body = error.body();
                    emit.error(body.error, &body.message);
                }
            }
        });
        Ok(accepted)
    }

    async fn finish(&self, cancel: bool) -> CoreResult<Ended> {
        let mut analyses = std::mem::take(&mut *lock(&self.analyses));
        if cancel {
            self.token.cancel();
        } else if tokio::time::timeout(STOP_WAIT, async {
            while analyses.join_next().await.is_some() {}
        })
        .await
        .is_err()
        {
            // What is left is cancelled; each stored draft is queued for later.
            self.token.cancel();
        }
        while analyses.join_next().await.is_some() {}
        self.workshop.finish(cancel).await.map_err(engine_error)?;
        Ok(Ended {
            status: if cancel {
                SessionStatus::Aborted
            } else {
                SessionStatus::Completed
            },
            feedback: None,
        })
    }
}

fn accepted_of(session_id: i64, submission: &DraftSubmission) -> DraftAccepted {
    DraftAccepted {
        session_id,
        turn_seq: submission.turn_seq,
        words: u32::try_from(submission.words).unwrap_or(u32::MAX),
        rule_checked: submission.rule_checked,
        rule_findings: submission.rule_findings.clone(),
        below_minimum: submission.below_minimum,
    }
}

// ---- graded reading ------------------------------------------------------------

pub(crate) struct ReadingRun {
    emit: Emitter,
    env: Env,
    topics: Option<Arc<tutor_engine::TopicBank>>,
    level: assessment_engine::Level,
    session: ReadingSession,
    token: CancellationToken,
}

pub(crate) async fn start_reading(
    shared: &Arc<Shared>,
    env: Env,
    request: &StartSessionRequest,
) -> CoreResult<Arc<ReadingRun>> {
    let level = level_of(request);
    let session = ReadingSession::start(
        ReadingDeps {
            client: env.llm.clone(),
            db: env.db.clone(),
            clock: env.clock.clone(),
        },
        ReadingConfig {
            profile_id: env.profile_id,
            provider_profile_id: env.provider_profile_id,
            model: env.model.clone(),
            level,
            first_language: env.first_language.clone(),
            app_version: env.app_version.clone(),
            word_levels: shared.catalogs.word_levels.clone(),
        },
    )
    .await
    .map_err(engine_error)?;
    let emit = Emitter::new(
        Arc::clone(&shared.bus),
        view_of(
            SessionKind::Reading,
            session.session_id(),
            SessionChannel::Text,
        ),
    );
    Ok(Arc::new(ReadingRun {
        emit,
        env,
        topics: shared.catalogs.topics.clone(),
        level,
        session,
        token: shared.shutdown.child_token(),
    }))
}

fn text_view(
    title: &str,
    passage: &str,
    glossary: &[tutor_engine::GlossaryEntry],
    questions: Vec<ReadingQuestionView>,
) -> ReadingTextView {
    ReadingTextView {
        title: title.to_owned(),
        passage: passage.to_owned(),
        glossary: glossary
            .iter()
            .map(|g| GlossEntryView {
                word: g.word.clone(),
                gloss_l1: g.gloss_l1.clone(),
                example: g.example.clone(),
            })
            .collect(),
        questions,
    }
}

fn generated_view(generated: &GeneratedReading) -> ReadingOutcomeView {
    let reading = &generated.reading;
    ReadingOutcomeView::Generated {
        content_id: generated.content_id,
        // The answers and the explanations are held back until the learner has
        // answered.
        text: text_view(
            &reading.title,
            &reading.passage,
            &reading.glossary,
            reading
                .questions
                .iter()
                .map(|q| ReadingQuestionView {
                    stem: q.stem.clone(),
                    options: q.options.clone(),
                })
                .collect(),
        ),
        regenerated: generated.regenerated,
        vocabulary_checked: generated.vocabulary_checked,
    }
}

impl ReadingRun {
    /// The units of the learner's level, for the authored fallback.
    async fn level_units(&self) -> CoreResult<Vec<Unit>> {
        let wanted = unit_level(self.level);
        let listed = self.env.core.list_units();
        let mut units = Vec::new();
        for summary in listed.units.iter().filter(|u| u.level == wanted) {
            units.push(self.env.core.unit(&summary.id).await?.unit);
        }
        Ok(units)
    }

    fn topic_choice(&self, topic: &TopicChoice) -> CoreResult<ReadingTopicChoice> {
        match topic {
            TopicChoice::Typed { text } => Ok(ReadingTopicChoice::Typed(text.clone())),
            TopicChoice::Bank { id } => {
                let bank = self.topics.as_ref().ok_or_else(|| {
                    CoreError::unavailable(
                        None,
                        "the topic bank is not installed (catalogs/topics.json), so only typed topics work",
                    )
                })?;
                bank.reading_topic(self.level, id)
                    .cloned()
                    .map(ReadingTopicChoice::Bank)
                    .ok_or(CoreError::NotFound {
                        what: "reading topic",
                    })
            }
        }
    }
}

#[async_trait]
impl Run for ReadingRun {
    fn emitter(&self) -> &Emitter {
        &self.emit
    }

    async fn generate_reading(&self, topic: TopicChoice) -> CoreResult<ReadingOutcomeView> {
        let choice = self.topic_choice(&topic)?;
        let token = self.token.child_token();
        let outcome = self
            .session
            .generate(&choice, &[], &token)
            .await
            .map_err(engine_error)?;
        match outcome {
            ReadingOutcome::Generated(generated) => Ok(generated_view(&generated)),
            ReadingOutcome::Fallback(fallback) => {
                // The engine was given no units, so it found no sets; they are
                // read here, only now that they are needed.
                let units = self.level_units().await?;
                let sets = authored_reading_sets(&units, self.level);
                Ok(ReadingOutcomeView::Fallback {
                    reason: match fallback.reason {
                        ReadingFallbackReason::ProviderUnavailable => {
                            ReadingFallbackReasonView::ProviderUnavailable
                        }
                        ReadingFallbackReason::Unusable => ReadingFallbackReasonView::Unusable,
                    },
                    sets: sets
                        .iter()
                        .map(|set| AuthoredSetView {
                            unit_id: set.unit_id.clone(),
                            activity_id: set.activity_id.clone(),
                            text: text_view(
                                &set.title,
                                &set.passage,
                                &[],
                                set.questions
                                    .iter()
                                    .map(|q| ReadingQuestionView {
                                        stem: q.stem.clone(),
                                        options: q.options.clone(),
                                    })
                                    .collect(),
                            ),
                        })
                        .collect(),
                })
            }
        }
    }

    async fn answer_reading(&self, request: AnswerReadingRequest) -> CoreResult<ReadingAnswered> {
        let answers: Vec<Option<usize>> = request
            .answers
            .iter()
            .map(|a| a.map(|n| usize::try_from(n).unwrap_or(usize::MAX)))
            .collect();
        let score = match &request.target {
            ReadingTarget::Generated { content_id } => self
                .session
                .score_generated(*content_id, &answers)
                .await
                .map_err(engine_error)?,
            ReadingTarget::Authored {
                unit_id,
                activity_id,
            } => {
                let units = self.level_units().await?;
                let set = authored_reading_sets(&units, self.level)
                    .into_iter()
                    .find(|s| &s.unit_id == unit_id && &s.activity_id == activity_id)
                    .ok_or(CoreError::NotFound {
                        what: "reading set",
                    })?;
                self.session
                    .score_authored(&set, &answers)
                    .await
                    .map_err(engine_error)?
            }
        };
        let view = reading_score(&score);
        self.emit.feedback(FeedbackView::Reading {
            score: view.clone(),
        });
        Ok(ReadingAnswered { score: view })
    }

    async fn finish(&self, cancel: bool) -> CoreResult<Ended> {
        self.token.cancel();
        self.session.finish(cancel).await.map_err(engine_error)?;
        Ok(Ended {
            status: if cancel {
                SessionStatus::Aborted
            } else {
                SessionStatus::Completed
            },
            feedback: None,
        })
    }
}
