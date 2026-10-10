//! The practice generation service: one structured call, the checks, one
//! regeneration, the fallback to authored items, storage, and scoring of the
//! learner's answers.

use std::sync::Arc;
use std::time::Instant;

use curriculum::validate::WordLevels;
use curriculum::{Activity, GeneratedType, Unit};
use llm_client::{ChatMessage, Contract, LlmClient, LlmError, StructuredRequest};
use serde_json::json;
use storage::{Attempt, AttemptOrigin, Database, GeneratedKind, LlmCallType, NewGeneratedContent};
use tokio_util::sync::CancellationToken;

use crate::activity::{Response, evidence_skill, score_deterministic};
use crate::error::{EngineError, Result};
use crate::evidence::{EvidenceRecorder, Subject};
use crate::support::{CallLog, Clock, assessment_level, assessment_origin};

use super::prompt::{PRACTICE_ITEMS_VERSION, PracticeContext, system_prompt, user_message};
use super::{RawItem, RawItems, Rejection, Vocabulary, check_item, item_text};

#[derive(Debug, Clone)]
pub struct PracticeConfig {
    pub profile_id: i64,
    /// The session the items are stored with and deleted with.
    pub session_id: i64,
    pub provider_profile_id: Option<i64>,
    pub model: String,
    /// The first language written in English ("Indonesian").
    pub first_language: String,
    /// Word levels for the vocabulary check. Without a list the check is skipped
    /// and the set says so.
    pub word_levels: Option<Arc<WordLevels>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PracticeSource {
    Generated,
    Authored,
}

/// Why the learner got authored items instead of generated ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackReason {
    /// The unit's `generation_policy` allows no generation.
    PolicyForbids,
    /// The session already holds as many generated items as the policy allows.
    SessionLimit,
    /// The provider could not be used: no provider is configured, or the call
    /// failed for a reason that is not about the model's output.
    ProviderUnavailable,
    /// The reply was unusable: it stayed invalid after the client's repair call.
    InvalidOutput,
    /// Fewer than half of the items passed the checks, twice.
    TooFewValid,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PracticeItem {
    pub activity: Activity,
    /// `Generated` for model items. `Authored` for replayed unit items, which are
    /// practice too: see [`record_practice_attempt`].
    pub origin: AttemptOrigin,
    pub grammar_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PracticeSet {
    pub items: Vec<PracticeItem>,
    pub source: PracticeSource,
    /// Why each dropped item was dropped, over both tries.
    pub rejected: Vec<Rejection>,
    pub regenerated: bool,
    pub fallback: Option<FallbackReason>,
    /// False when no word list was available for the vocabulary check.
    pub vocabulary_checked: bool,
}

pub struct PracticeGenerator {
    config: PracticeConfig,
    client: Arc<dyn LlmClient>,
    db: Database,
    clock: Clock,
}

struct Attempted {
    kept: Vec<(Activity, RawItem)>,
    rejected: Vec<Rejection>,
}

fn is_indonesian(name: &str) -> bool {
    name.trim().eq_ignore_ascii_case("indonesian")
}

fn raw_text(raw: &RawItem) -> &str {
    match raw.kind {
        GeneratedType::Mcq => &raw.stem,
        GeneratedType::GapFill => &raw.text,
        GeneratedType::Reorder => &raw.answer,
    }
}

impl PracticeGenerator {
    pub fn new(
        config: PracticeConfig,
        client: Arc<dyn LlmClient>,
        db: Database,
        clock: Clock,
    ) -> Self {
        Self {
            config,
            client,
            db,
            clock,
        }
    }

    /// Items stored earlier in this session: how many, and their text.
    async fn earlier(&self) -> Result<(usize, Vec<String>)> {
        let rows = self
            .db
            .generated_content()
            .for_session(self.config.session_id)
            .await?;
        let mut count = 0;
        let mut texts = Vec::new();
        for row in rows
            .iter()
            .filter(|r| r.kind == GeneratedKind::PracticeItems)
        {
            if let Ok(parsed) = serde_json::from_value::<RawItems>(row.content.clone()) {
                count += parsed.items.len();
                texts.extend(parsed.items.iter().map(|i| raw_text(i).to_owned()));
            }
        }
        Ok((count, texts))
    }

    /// Up to `count` extra items for `unit`. `wrong_activity_ids` are the authored
    /// items the learner got wrong, replayed first when generation is not possible.
    ///
    /// A provider that is down, bad output and too few valid items all end in
    /// authored items rather than an error; the set says why. Only a cancelled
    /// call and storage failures are errors.
    pub async fn generate(
        &self,
        unit: &Unit,
        count: usize,
        wrong_activity_ids: &[String],
        cancel: &CancellationToken,
    ) -> Result<PracticeSet> {
        let policy = &unit.generation_policy;
        let vocabulary = Vocabulary::for_unit(unit, self.config.word_levels.clone());
        let fallback =
            |reason: FallbackReason, rejected: Vec<Rejection>, regenerated: bool| PracticeSet {
                items: authored_fallback(unit, wrong_activity_ids, count),
                source: PracticeSource::Authored,
                rejected,
                regenerated,
                fallback: Some(reason),
                vocabulary_checked: vocabulary.is_checked(),
            };
        if policy.allowed_types.is_empty() || policy.max_items_per_session == 0 {
            return Ok(fallback(FallbackReason::PolicyForbids, Vec::new(), false));
        }
        let (already, earlier_texts) = self.earlier().await?;
        let room = usize::from(policy.max_items_per_session).saturating_sub(already);
        let count = count.min(room);
        if count == 0 {
            return Ok(fallback(FallbackReason::SessionLimit, Vec::new(), false));
        }

        let mut rejected = Vec::new();
        let mut best: Option<Attempted> = None;
        let mut regenerated = false;
        for attempt in 0..2 {
            regenerated = attempt == 1;
            match self
                .attempt(unit, count, already, &earlier_texts, &vocabulary, cancel)
                .await
            {
                Ok(done) => {
                    rejected.extend(done.rejected.iter().cloned());
                    let enough = done.kept.len() * 2 >= count && !done.kept.is_empty();
                    let better = best.as_ref().is_none_or(|b| done.kept.len() > b.kept.len());
                    if enough {
                        best = Some(done);
                        break;
                    }
                    if better {
                        best = Some(done);
                    }
                }
                Err(EngineError::Llm(LlmError::Cancelled)) => {
                    return Err(EngineError::Llm(LlmError::Cancelled));
                }
                Err(EngineError::Llm(error)) => {
                    // Only invalid output points at the model's answer being
                    // unusable. Everything else is the provider being unusable
                    // for this call: no provider configured (the `NoProvider`
                    // client answers `InvalidRequest`), a rejected key, a
                    // timeout, a rate limit, a refusal. The reading service
                    // makes the same split.
                    let reason = match error {
                        LlmError::InvalidOutput(_) => FallbackReason::InvalidOutput,
                        _ => FallbackReason::ProviderUnavailable,
                    };
                    return Ok(fallback(reason, rejected, regenerated));
                }
                Err(EngineError::Output(_)) => {
                    return Ok(fallback(
                        FallbackReason::InvalidOutput,
                        rejected,
                        regenerated,
                    ));
                }
                Err(other) => return Err(other),
            }
        }

        let Some(kept) = best.filter(|b| !b.kept.is_empty() && b.kept.len() * 2 >= count) else {
            return Ok(fallback(FallbackReason::TooFewValid, rejected, regenerated));
        };
        let raws: Vec<&RawItem> = kept.kept.iter().map(|(_, raw)| raw).collect();
        self.db
            .generated_content()
            .add(&NewGeneratedContent {
                session_id: self.config.session_id,
                kind: GeneratedKind::PracticeItems,
                content: json!({ "items": raws }),
                contract_version: PRACTICE_ITEMS_VERSION.to_owned(),
                model: self.config.model.clone(),
                created_at: (self.clock)(),
            })
            .await?;
        let items = kept
            .kept
            .into_iter()
            .map(|(activity, raw)| PracticeItem {
                activity,
                origin: AttemptOrigin::Generated,
                grammar_id: Some(raw.grammar_id),
            })
            .collect();
        Ok(PracticeSet {
            items,
            source: PracticeSource::Generated,
            rejected,
            regenerated,
            fallback: None,
            vocabulary_checked: vocabulary.is_checked(),
        })
    }

    /// One call and the checks on what came back.
    async fn attempt(
        &self,
        unit: &Unit,
        count: usize,
        already: usize,
        earlier_texts: &[String],
        vocabulary: &Vocabulary,
        cancel: &CancellationToken,
    ) -> Result<Attempted> {
        let policy = &unit.generation_policy;
        let context = PracticeContext::from_unit(unit, earlier_texts);
        let system = system_prompt(
            count,
            &policy.allowed_types,
            assessment_level(policy.max_level),
            &self.config.first_language,
        );
        let request = StructuredRequest::new(
            Contract::PracticeItems,
            system,
            vec![ChatMessage::user(user_message(&context))],
            u32::try_from(300 + 250 * count).unwrap_or(2000),
        )
        .with_temperature(0.0);

        let started_at = (self.clock)();
        let timer = Instant::now();
        let result = self.client.structured(request, cancel.clone()).await;
        CallLog {
            db: &self.db,
            provider_profile_id: self.config.provider_profile_id,
            call_type: LlmCallType::PracticeGen,
            model: &self.config.model,
            started_at,
            elapsed: timer.elapsed(),
        }
        .structured(&result)
        .await;
        let output = result?;
        let parsed: RawItems = serde_json::from_value(output.value)
            .map_err(|_| EngineError::Output("practice_items"))?;

        let indonesian = is_indonesian(&self.config.first_language);
        let mut existing: Vec<String> = context.existing_items.clone();
        let mut kept: Vec<(Activity, RawItem)> = Vec::new();
        let mut rejected = Vec::new();
        for raw in parsed.items {
            if kept.len() == count {
                break;
            }
            let id = format!(
                "gen-s{}-{}",
                self.config.session_id,
                already + kept.len() + 1
            );
            match check_item(&raw, id, unit, &existing, vocabulary, indonesian) {
                Ok(activity) => {
                    if let Some(text) = item_text(&activity) {
                        existing.push(text);
                    }
                    kept.push((activity, raw));
                }
                Err(reason) => rejected.push(reason),
            }
        }
        Ok(Attempted { kept, rejected })
    }
}

/// Authored items to practise with when generation is not possible: the ones the
/// learner got wrong first, in the order given, then the other authored items
/// of an allowed type. At most `limit`.
pub fn authored_fallback(
    unit: &Unit,
    wrong_activity_ids: &[String],
    limit: usize,
) -> Vec<PracticeItem> {
    let allowed = &unit.generation_policy.allowed_types;
    let usable = |activity: &Activity| -> bool {
        let kind = match activity {
            Activity::Mcq(_) => GeneratedType::Mcq,
            Activity::GapFill(_) => GeneratedType::GapFill,
            Activity::Reorder(_) => GeneratedType::Reorder,
            _ => return false,
        };
        // An mcq that reads a passage or plays audio needs that context, which a
        // single replayed item does not bring.
        let standalone = match activity {
            Activity::Mcq(a) => a.passage.is_none() && a.audio_text.is_none(),
            _ => true,
        };
        allowed.contains(&kind) && standalone
    };
    let mut ordered: Vec<&Activity> = Vec::new();
    for id in wrong_activity_ids {
        if let Some(found) = unit.activities.iter().find(|a| a.id() == id.as_str())
            && usable(found)
            && !ordered.iter().any(|a| a.id() == found.id())
        {
            ordered.push(found);
        }
    }
    for activity in &unit.activities {
        if usable(activity) && !ordered.iter().any(|a| a.id() == activity.id()) {
            ordered.push(activity);
        }
    }
    ordered
        .into_iter()
        .take(limit)
        .map(|activity| PracticeItem {
            activity: activity.clone(),
            origin: AttemptOrigin::Authored,
            grammar_id: None,
        })
        .collect()
}

/// What the learner answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PracticeAnswer {
    /// The index of the chosen option of an mcq.
    Choice(usize),
    /// One answer per gap.
    Gaps(Vec<String>),
    /// The tokens in the order the learner put them.
    Order(Vec<String>),
}

/// Scores an answer to a practice item and stores the attempt.
///
/// The attempt never counts toward an estimate, whatever the item's origin. A
/// generated item is not reviewed content. A replayed authored item was already
/// counted when it was first answered in the unit, and counting it again would
/// let a learner raise an estimate by repeating the same item; so replays are
/// stored with their authored origin and `counts_toward_estimate = false`.
///
/// The score is the one of the activity runtime ([`crate::score_deterministic`]),
/// so a practice answer and a unit answer are marked by the same code, and the
/// row is written by the recorder like every other.
pub async fn record_practice_attempt(
    recorder: &EvidenceRecorder,
    profile_id: i64,
    session_id: i64,
    unit: &Unit,
    item: &PracticeItem,
    answer: &PracticeAnswer,
) -> Result<(f64, Attempt)> {
    let response = match answer {
        PracticeAnswer::Choice(index) => Response::Choice(*index),
        PracticeAnswer::Gaps(given) => Response::Gaps(given.clone()),
        PracticeAnswer::Order(given) => Response::Order(given.clone()),
    };
    let scored = score_deterministic(&item.activity, &response)?;
    let activity_id = item.activity.id().to_owned();
    let subject = Subject::new(
        profile_id,
        Some(session_id),
        Some(unit.id.clone()),
        activity_id.clone(),
        item.activity.activity_type().as_str(),
        assessment_level(unit.level),
        evidence_skill(&item.activity, false),
        assessment_origin(item.origin),
        recorder.new_response_id(&activity_id),
    )
    .never_counts();
    let recorded = recorder
        .record_deterministic(&subject, &scored.record)
        .await?;
    let attempt = recorded
        .attempts
        .into_iter()
        .next()
        .ok_or(EngineError::Refused("no attempt was stored"))?;
    Ok((scored.record.normalized, attempt))
}
