//! The `roleplay` activity: a conversation with the tutor in a role, on the text
//! channel, through the same conversation engine as the free chat.
//!
//! The roleplay runs inside the unit's lesson session, so its turns and their
//! analysis are stored with the rest of the run and deleted with it. The number
//! of learner turns is bounded by the activity's `max_turns`; the tutor's replies
//! are streamed text, limited by level (contract section 7). When the activity's
//! scoring is `rubric`, the learner's lines are scored with the interaction
//! rubric of the unit's level; otherwise the roleplay is practice and one
//! unscored attempt records that it was done. A roleplay by voice runs through the
//! voice loop of `app-core`, not here.

use curriculum::{Activity, Roleplay, RoleplayMode, Scoring};
use storage::{AttemptStatus, Scorer};
use tokio_util::sync::CancellationToken;

use super::error::ActivityError;
use super::player::{ActivityResult, ResultOutcome, UnitPlayer, UnscoredReason};
use super::productive::outcome_scores;
use crate::analysis::ObjectiveRef;
use crate::chat::{ChatConfig, ChatReply, ChatTopic, TextChat, UnitRoleplay};
use crate::error::Result;
use crate::prompt::FeedbackMode;
use crate::rubric::{InputMode, Runs, ScoreRequest, ScoreResult, TaskFamily, WorkshopTask};
use crate::session::{EndReason, Phase};
use crate::support::assessment_level;

/// A roleplay in progress.
pub struct RoleplayRun {
    chat: TextChat,
    activity_id: String,
    max_turns: u8,
    learner_texts: Vec<String>,
    scenario: String,
    goals: Vec<String>,
}

impl RoleplayRun {
    /// Lets `observer` hear about stored messages and finished analyses.
    #[must_use]
    pub fn with_observer(mut self, observer: std::sync::Arc<dyn crate::ChatObserver>) -> Self {
        self.chat = self.chat.with_observer(observer);
        self
    }

    /// The tutor speaks first.
    pub async fn open(
        &mut self,
        on_delta: impl FnMut(&str) + Send,
        cancel: &CancellationToken,
    ) -> Result<ChatReply> {
        self.chat.open(on_delta, cancel).await
    }

    /// One learner turn. Refused once the activity's turns are used.
    pub async fn say(
        &mut self,
        text: &str,
        on_delta: impl FnMut(&str) + Send,
        cancel: &CancellationToken,
    ) -> Result<ChatReply> {
        if self.turns_left() == 0 {
            return Err(ActivityError::NoTurnsLeft {
                max: self.max_turns,
            }
            .into());
        }
        let reply = self.chat.send(text, on_delta, cancel).await?;
        self.learner_texts.push(text.trim().to_owned());
        Ok(reply)
    }

    /// Learner turns left before the roleplay is over.
    pub fn turns_left(&self) -> u8 {
        let used = u8::try_from(self.learner_texts.len()).unwrap_or(u8::MAX);
        self.max_turns.saturating_sub(used)
    }

    pub fn phase(&self) -> Phase {
        self.chat.phase()
    }

    pub fn activity_id(&self) -> &str {
        &self.activity_id
    }
}

fn vocabulary_and_grammar(unit: &curriculum::Unit, a: &Roleplay) -> Vec<String> {
    let mut out = Vec::new();
    for id in &a.target_vocab_ids {
        if let Some(item) = unit.targets.vocabulary.iter().find(|v| &v.id == id) {
            out.push(item.lemma.clone());
        }
    }
    for id in &a.target_grammar_ids {
        if let Some(point) = unit.targets.grammar.iter().find(|g| &g.id == id) {
            out.push(format!("{}: {}", point.name, point.pattern));
        }
    }
    out
}

impl UnitPlayer {
    /// Starts the roleplay `id` of the unit.
    pub fn start_roleplay(&mut self, id: &str) -> Result<RoleplayRun> {
        self.check_open(id)?;
        let Activity::Roleplay(a) = self.activity(id)?.clone() else {
            return Err(ActivityError::WrongKind {
                expected: "a roleplay activity",
            }
            .into());
        };
        let objectives = a
            .objective_ids
            .iter()
            .filter_map(|objective| self.unit.objectives.iter().find(|o| &o.id == objective))
            .map(|o| ObjectiveRef {
                id: o.id.clone(),
                can_do: o.can_do.en.clone(),
            })
            .collect();
        let scenario = a.scenario.en.clone();
        let config = ChatConfig {
            profile_id: self.config.profile_id,
            provider_profile_id: self.env.provider_profile_id,
            model: self.env.model.clone(),
            level: assessment_level(self.unit.level),
            first_language: self.config.first_language.clone(),
            mode: match a.mode {
                RoleplayMode::Fluency => FeedbackMode::Fluency,
                RoleplayMode::Accuracy => FeedbackMode::Accuracy,
            },
            topic: ChatTopic::Unit(UnitRoleplay {
                unit_id: self.unit.id.clone(),
                unit_title: self.unit.title.en.clone(),
                activity_id: a.id.clone(),
                scenario: scenario.clone(),
                tutor_role: a.tutor_role.clone(),
                learner_role: a.learner_role.clone(),
                goals: a.goals.clone(),
                target_language: vocabulary_and_grammar(&self.unit, &a),
                objectives,
            }),
            app_version: self.config.app_version.clone(),
        };
        let chat = TextChat::start_in(self.env.chat_deps(), config, self.session_id)?;
        Ok(RoleplayRun {
            chat,
            activity_id: a.id.clone(),
            max_turns: a.max_turns,
            learner_texts: Vec::new(),
            scenario,
            goals: a.goals.clone(),
        })
    }

    /// Ends a roleplay: the turn analysis is finished, then the learner's lines
    /// are scored when the activity has a rubric scoring, or recorded as an
    /// unscored attempt when it is practice.
    pub async fn finish_roleplay(
        &mut self,
        mut run: RoleplayRun,
        cancel: &CancellationToken,
    ) -> Result<ActivityResult> {
        let Activity::Roleplay(a) = self.activity(&run.activity_id)?.clone() else {
            return Err(ActivityError::WrongKind {
                expected: "a roleplay activity",
            }
            .into());
        };
        run.chat.finish(EndReason::Finished, cancel).await?;
        let activity = Activity::Roleplay(a.clone());
        let skill = "speaking";
        let subject = self.subject(&activity, skill);
        let response_id = subject.response_id.clone();
        let text = run.learner_texts.join("\n");

        let rubric = (a.scoring == Scoring::Rubric)
            .then(|| {
                self.env
                    .rubrics
                    .for_family(
                        assessment_level(self.unit.level),
                        TaskFamily::SpokenInteraction,
                    )
                    .map(|r| r.rubric.clone())
            })
            .flatten();
        if a.scoring == Scoring::Rubric && rubric.is_none() {
            self.recorder
                .record_unscored(
                    &subject,
                    Scorer::RubricLlm,
                    "rubric_missing/1",
                    "overall",
                    AttemptStatus::Insufficient,
                    "The interaction rubric of this level is not in the catalog.",
                    Some(&text),
                    serde_json::json!({ "turns": run.learner_texts.len() }),
                )
                .await?;
            return self
                .finish_activity(
                    &activity,
                    skill,
                    None,
                    None,
                    ResultOutcome::Unscored(UnscoredReason::InteractionRubricMissing),
                    vec![response_id],
                )
                .await;
        }
        let (Some(rubric), false) = (rubric, text.trim().is_empty()) else {
            // Practice, or a roleplay in which the learner said nothing.
            let reason = if a.scoring == Scoring::Rubric {
                "The learner took no turn, so there is nothing to score."
            } else {
                "A roleplay with no scorer is practice."
            };
            self.recorder
                .record_unscored(
                    &subject,
                    Scorer::Deterministic,
                    "roleplay/1",
                    "completion",
                    AttemptStatus::Insufficient,
                    reason,
                    (!text.is_empty()).then_some(text.as_str()),
                    serde_json::json!({ "turns": run.learner_texts.len() }),
                )
                .await?;
            return self
                .finish_activity(
                    &activity,
                    skill,
                    None,
                    None,
                    ResultOutcome::Unscored(UnscoredReason::NoScorer),
                    vec![response_id],
                )
                .await;
        };
        let request = ScoreRequest {
            subject,
            rubric,
            task: WorkshopTask {
                prompt: run.scenario.clone(),
                content_points: run.goals.clone(),
                min_words: None,
            },
            response: text,
            input_mode: InputMode::Text,
            runs: if self.in_checkpoint(&run.activity_id) {
                Runs::Two
            } else {
                Runs::One
            },
            first_language: self.config.first_language.clone(),
            timing: None,
        };
        match self.scorer.score(request, cancel).await? {
            ScoreResult::Scored { outcome, .. } => {
                let (score, confidence) = outcome_scores(&outcome);
                self.finish_activity(
                    &activity,
                    skill,
                    score,
                    confidence,
                    ResultOutcome::Rubric(outcome),
                    vec![response_id],
                )
                .await
            }
            ScoreResult::Queued { .. } => {
                self.finish_activity(
                    &activity,
                    skill,
                    None,
                    None,
                    ResultOutcome::Queued,
                    vec![response_id],
                )
                .await
            }
        }
    }
}
