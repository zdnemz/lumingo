//! Turn analysis (call type T2, `PROMPT_CONTRACTS.md` section 6): the system
//! prompt and request of one structured analysis call, the semantic filters
//! that run after schema validation, the cadence (every turn; every third turn
//! after a rate limit; flushed at session end) and the reliability window.
//!
//! No storage and no spawning live here: the caller runs [`run_analysis`] after
//! the tutor's reply has finished, writes the outcome through `storage`, and
//! feeds [`FilteredAnalysis::notes`] into the next T1 turn. Fake clients in
//! tests are the only non-live callers.

use std::collections::{HashSet, VecDeque};

use curriculum::{Activity, Level, Unit};
use llm_client::{LlmError, StructuredRequest};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::llm::LlmClient;

/// The prompt version of T2 (PROMPT_CONTRACTS section 11).
pub const TURN_ANALYSIS_VERSION: &str = "turn_analysis/1";

/// The error cap of a conversation turn or chat message (T2).
pub const CONVERSATION_ERROR_CAP: u32 = 5;

/// The error cap of a writing draft (T2).
pub const DRAFT_ERROR_CAP: u32 = 20;

/// How many turns one batched call carries after a rate limit (T2: "every
/// third turn").
pub const BATCH_SIZE: usize = 3;

/// The schema/tool name of the structured call.
const SCHEMA_NAME: &str = "turn_analysis";

/// The output budget of one T2 call. The contract sets none; this is a
/// starting value covering a batch of three turns with the full error cap.
const MAX_TOKENS: u32 = 1500;

/// T2 runs at temperature 0 (PROMPT_CONTRACTS section 8).
const TEMPERATURE: f32 = 0.0;

/// The reliability window: the last 20 analyses (T2).
const WINDOW: usize = 20;

/// How many analyses the window needs before it may say "unreliable". Without
/// a floor, one small analysis with one dropped entry would be enough; the
/// contract's "last 20 analyses" implies a sample, and five is the smallest
/// sample that can exceed a third.
const MIN_ANALYSES: usize = 5;

const CONTRACT: &str = include_str!("../../../contracts/turn_analysis.schema.json");

/// The bundled T2 contract, parsed. A failure here is a build defect; the test
/// suite checks it, so the expect is confined to this one spot.
#[allow(clippy::expect_used)]
fn contract_schema() -> Value {
    serde_json::from_str(CONTRACT).expect("bundled turn_analysis contract is valid JSON")
}

/// How the learner produced the text of a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputMode {
    Voice,
    Text,
}

/// One objective of the session, as T2 wants it in its input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectivePair {
    pub id: String,
    pub can_do: String,
}

/// One learner turn to analyse. `turn_seq` is the turn's sequence number in its
/// session, which the model echoes back and the caller maps to a stored turn.
/// For a writing draft, `tutor_before` and `tutor_reply` are empty (T2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalysisTurn {
    pub turn_seq: i64,
    pub tutor_before: String,
    pub learner_text: String,
    pub tutor_reply: String,
}

impl AnalysisTurn {
    /// One writing draft as T2 reads it: a draft has no tutor text before or
    /// after it, and its input mode is text (T2, writing workshop).
    pub fn draft(turn_seq: i64, text: &str) -> Self {
        Self {
            turn_seq,
            tutor_before: String::new(),
            learner_text: text.to_owned(),
            tutor_reply: String::new(),
        }
    }
}

/// Everything one T2 call carries (T2's user message). Serialises to the JSON
/// object the contract describes: `level`, `l1`, `input_mode`, `objectives`,
/// `target_language`, `turns`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalysisInput {
    pub level: Level,
    pub l1: String,
    pub input_mode: InputMode,
    pub objectives: Vec<ObjectivePair>,
    pub target_language: Vec<String>,
    pub turns: Vec<AnalysisTurn>,
}

/// Why a unit could not provide an activity for analysis.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ActivityError {
    #[error("the unit has no activity")]
    NoActivity,
    #[error("the unit has no activity with the id `{0}`")]
    UnknownActivity(String),
}

impl AnalysisInput {
    /// The input of one activity of `unit`: the activity's objective ids
    /// resolved to their can-do text in the stored `<unit id>/<objective id>`
    /// form, and, for a roleplay, its target language (the same list T1 gets).
    pub fn from_activity(
        unit: &Unit,
        activity_id: Option<&str>,
        input_mode: InputMode,
        l1: &str,
        turns: Vec<AnalysisTurn>,
    ) -> Result<Self, ActivityError> {
        let activity = unit
            .activities
            .iter()
            .find(|activity| activity_id.is_none_or(|id| activity.common().id == id))
            .ok_or_else(|| match activity_id {
                Some(id) => ActivityError::UnknownActivity(id.to_owned()),
                None => ActivityError::NoActivity,
            })?;
        let objective_ids = &activity.common().objective_ids;
        let objectives = unit
            .objectives
            .iter()
            .filter(|objective| objective_ids.contains(&objective.id))
            .map(|objective| ObjectivePair {
                id: format!("{}/{}", unit.id, objective.id),
                can_do: objective.can_do.en.clone(),
            })
            .collect();
        let target_language = match activity {
            Activity::Roleplay {
                target_grammar_ids,
                target_vocab_ids,
                ..
            } => {
                crate::prompt::activity_target_language(unit, target_grammar_ids, target_vocab_ids)
            }
            _ => Vec::new(),
        };
        Ok(Self {
            level: unit.level,
            l1: l1.to_owned(),
            input_mode,
            objectives,
            target_language,
            turns,
        })
    }

    /// The input of a free conversation: no unit objectives, no target list.
    pub fn free(level: Level, input_mode: InputMode, l1: &str, turns: Vec<AnalysisTurn>) -> Self {
        Self {
            level,
            l1: l1.to_owned(),
            input_mode,
            objectives: Vec::new(),
            target_language: Vec::new(),
            turns,
        }
    }
}

/// One error the model reported, already filtered. The `category`, `severity`
/// and status values are the schema's enums; validation happened in
/// `llm-client` before these were built, so they stay strings here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorFinding {
    pub category: String,
    pub quote: String,
    pub correction: String,
    pub severity: String,
    pub addressed_in_reply: bool,
}

/// One piece of objective evidence the model reported, already filtered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectiveEvidence {
    pub objective_id: String,
    pub status: String,
    pub quote: String,
}

/// The analysis of one turn, in the contract's shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnAnalysisEntry {
    pub turn_seq: i64,
    pub errors: Vec<ErrorFinding>,
    pub objective_evidence: Vec<ObjectiveEvidence>,
    pub understood_tutor: String,
    pub note_for_next_turn: String,
}

/// What one analysis call produced and what the filters removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DropCounts {
    /// Error and evidence entries the model returned.
    pub produced: usize,
    /// Entries the filters removed, for any reason.
    pub dropped: usize,
}

/// The kept analysis of a call, plus the drop counts of its filters.
#[derive(Debug, Clone, PartialEq)]
pub struct FilteredAnalysis {
    pub turns: Vec<TurnAnalysisEntry>,
    pub counts: DropCounts,
}

impl FilteredAnalysis {
    /// The `note_for_next_turn` values, in turn order, skipping empty ones.
    /// T1 caps the list it actually sends (`MAX_NOTES`).
    pub fn notes(&self) -> Vec<String> {
        self.turns
            .iter()
            .map(|turn| turn.note_for_next_turn.trim().to_owned())
            .filter(|note| !note.is_empty())
            .collect()
    }

    /// The contract-shaped JSON for `turn_analysis.analysis_json`.
    pub fn analysis_json(&self) -> Result<String, AnalysisFailure> {
        #[derive(Serialize)]
        struct Stored<'a> {
            turns: &'a [TurnAnalysisEntry],
        }
        serde_json::to_string(&Stored { turns: &self.turns })
            .map_err(|error| AnalysisFailure::Malformed(error.to_string()))
    }
}

/// Why an analysis run did not produce a usable outcome.
#[derive(Debug, thiserror::Error)]
pub enum AnalysisFailure {
    /// The provider call failed. A rate limit is the caller's signal to switch
    /// the cadence to batched.
    #[error(transparent)]
    Provider(#[from] LlmError),
    /// The validated value did not fit the contract types, or could not be
    /// serialised. This is a bug or a contract drift, not a provider problem.
    #[error("the analysis did not fit the contract types: {0}")]
    Malformed(String),
}

/// The system prompt of T2, with the call's error cap filled in.
pub fn analysis_system_prompt(max_errors: u32) -> String {
    format!(
        "You analyse turns from an English learner for a tutoring app. Return only JSON that matches the schema.\n\
         \n\
         For each turn in the input:\n\
         \n\
         errors\n\
         - List real language errors in the learner's text. If there are none, return an empty list. Do not invent errors.\n\
         - \"quote\" must be copied exactly from the learner's text. \"correction\" is the smallest change that fixes it.\n\
         - \"category\" must be one of the allowed values. Use \"l1_transfer\" only when the error clearly follows a pattern of the learner's first language.\n\
         - \"severity\": \"minor\" if meaning is clear and a listener would hardly notice, \"major\" if it is clearly wrong but understandable, \"blocking\" if the meaning is lost.\n\
         - If input_mode is \"voice\", the text is a speech transcript: ignore spelling, capital letters, and punctuation.\n\
         - Judge the learner at the stated level. Simple language is not an error.\n\
         - \"addressed_in_reply\" is true if the tutor's reply corrected the error or used the correct form of the same phrase.\n\
         - At most {max_errors} errors per turn, most severe first.\n\
         \n\
         objective_evidence\n\
         - Use only objective ids given in the input. Include an objective only if the turn shows something about it.\n\
         - \"status\": \"demonstrated\", \"partial\", or \"not_demonstrated\". \"quote\" is copied exactly from the learner's text, or empty for \"not_demonstrated\".\n\
         \n\
         understood_tutor\n\
         - Whether the learner's turn shows they understood the tutor's previous message: \"yes\", \"partly\", \"no\", or \"not_applicable\" when there was no previous message.\n\
         \n\
         note_for_next_turn\n\
         - One short sentence the tutor can use next, at most 25 words. Empty if nothing is worth noting."
    )
}

/// The request of one T2 call: the schema is the bundled contract, the user
/// message is the input JSON, and the temperature is 0.
pub fn analysis_request(
    input: &AnalysisInput,
    max_errors: u32,
) -> Result<StructuredRequest, AnalysisFailure> {
    let content = serde_json::to_string(input)
        .map_err(|error| AnalysisFailure::Malformed(error.to_string()))?;
    Ok(StructuredRequest {
        system: Some(analysis_system_prompt(max_errors)),
        messages: vec![llm_client::Message {
            role: llm_client::Role::User,
            content,
        }],
        schema_name: SCHEMA_NAME.to_owned(),
        schema: contract_schema(),
        max_tokens: MAX_TOKENS,
        temperature: Some(TEMPERATURE),
    })
}

/// One structured T2 call: build the request, call the client, filter what
/// comes back. The value was schema-validated by the client already.
pub async fn run_analysis(
    client: &dyn LlmClient,
    input: &AnalysisInput,
    max_errors: u32,
    cancel: &CancellationToken,
) -> Result<AnalysisOutcome, AnalysisFailure> {
    let request = analysis_request(input, max_errors)?;
    let output = client.structured(request, cancel.clone()).await?;
    let filtered = filter_output(&output.value, input, max_errors as usize)?;
    Ok(AnalysisOutcome {
        filtered,
        ladder_level: output.level.number(),
        repaired: output.repaired,
    })
}

/// What one successful analysis run produced.
#[derive(Debug, Clone, PartialEq)]
pub struct AnalysisOutcome {
    pub filtered: FilteredAnalysis,
    /// The ladder level the client used (1 to 4), for `turn_analysis`.
    pub ladder_level: u8,
    /// Whether the first answer was invalid and the repair call fixed it.
    pub repaired: bool,
}

/// Applies T2's semantic filters to a schema-validated value: drop an error
/// whose quote is not in the learner's text (case and whitespace are
/// normalised first, and an empty quote is a mismatch), drop `spelling` and
/// `punctuation` on voice turns, drop evidence for objectives that were not in
/// the input or whose non-empty quote is not in the text, drop entries for
/// unknown or repeated turn numbers, and keep at most `max_errors` errors per
/// turn, most severe first as the model ordered them.
pub fn filter_output(
    value: &Value,
    input: &AnalysisInput,
    max_errors: usize,
) -> Result<FilteredAnalysis, AnalysisFailure> {
    #[derive(Deserialize)]
    struct Raw {
        turns: Vec<TurnAnalysisEntry>,
    }
    let raw: Raw = serde_json::from_value(value.clone())
        .map_err(|error| AnalysisFailure::Malformed(error.to_string()))?;

    let known: Vec<&AnalysisTurn> = input.turns.iter().collect();
    let objective_ids: HashSet<&str> = input
        .objectives
        .iter()
        .map(|objective| objective.id.as_str())
        .collect();

    let mut counts = DropCounts::default();
    let mut seen: HashSet<i64> = HashSet::new();
    let mut turns = Vec::new();

    for mut entry in raw.turns {
        counts.produced += entry.errors.len() + entry.objective_evidence.len();
        let Some(turn) = known
            .iter()
            .find(|turn| turn.turn_seq == entry.turn_seq)
            .filter(|_| seen.insert(entry.turn_seq))
        else {
            // Unknown, or a second entry for a turn already consumed: nothing
            // here can be stored against a turn.
            counts.dropped += entry.errors.len() + entry.objective_evidence.len();
            continue;
        };
        let text = normalise(&turn.learner_text);
        let voice = input.input_mode == InputMode::Voice;

        let before = entry.errors.len();
        entry.errors.retain(|error| {
            if voice && matches!(error.category.as_str(), "spelling" | "punctuation") {
                return false;
            }
            let quote = normalise(&error.quote);
            !quote.is_empty() && text.contains(&quote)
        });
        // The cap keeps the model's own order, which the prompt fixes as most
        // severe first.
        entry.errors.truncate(max_errors);
        counts.dropped += before - entry.errors.len();

        let before = entry.objective_evidence.len();
        entry.objective_evidence.retain(|evidence| {
            if !objective_ids.contains(evidence.objective_id.as_str()) {
                return false;
            }
            let quote = normalise(&evidence.quote);
            quote.is_empty() || text.contains(&quote)
        });
        counts.dropped += before - entry.objective_evidence.len();

        turns.push(entry);
    }

    Ok(FilteredAnalysis { turns, counts })
}

/// Lowercased, whitespace-collapsed text, the form T2's substring filters use.
/// The writing workshop's revision comparison shares it, so two quotes the
/// filter would call equal compare equal.
pub(crate) fn normalise(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// The cadence of T2 calls in one session: every turn normally; after the
/// provider rate-limited a call, one call per [`BATCH_SIZE`] turns; whatever is
/// left is flushed at session end. Nothing is ever dropped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AnalysisCadence {
    batched: bool,
    pending: VecDeque<i64>,
}

impl AnalysisCadence {
    pub fn new() -> Self {
        Self::default()
    }

    /// True once a rate limit switched the cadence to batched.
    pub fn is_batched(&self) -> bool {
        self.batched
    }

    /// Turn numbers waiting for analysis.
    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    /// Queues one learner turn. A repeated number is kept once.
    pub fn enqueue(&mut self, turn_seq: i64) {
        if !self.pending.contains(&turn_seq) {
            self.pending.push_back(turn_seq);
        }
    }

    /// A call came back rate-limited: from now on the session analyses every
    /// third turn instead of every turn.
    pub fn note_rate_limited(&mut self) {
        self.batched = true;
    }

    /// The turn numbers one call right now would carry: everything pending in
    /// normal cadence; in batched cadence the oldest [`BATCH_SIZE`] turns, but
    /// only once that many have gathered, so a rate-limited session really
    /// analyses one turn in three instead of one per turn.
    pub fn due(&self) -> Vec<i64> {
        if self.batched {
            if self.pending.len() >= BATCH_SIZE {
                self.pending.iter().take(BATCH_SIZE).copied().collect()
            } else {
                Vec::new()
            }
        } else {
            self.pending.iter().copied().collect()
        }
    }

    /// The turn numbers a session-end flush carries: everything left.
    pub fn flush(&self) -> Vec<i64> {
        self.pending.iter().copied().collect()
    }

    /// Removes the turns of a call that completed (whatever it reported, they
    /// are analysed). Turns of a failed call stay pending for the next due or
    /// the flush.
    pub fn mark_analysed(&mut self, turn_seqs: &[i64]) {
        self.pending.retain(|seq| !turn_seqs.contains(seq));
    }
}

/// The latest notes from T2, ready for the next T1 turn: the most recent
/// analysis's non-empty `note_for_next_turn` values, at most [`MAX_NOTES`] of
/// them (T1 takes the same cap when it builds the turn message).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NotesForNextTurn {
    notes: Vec<String>,
}

impl NotesForNextTurn {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the buffer with the notes of the newest analysis. Notes from an
    /// older analysis are stale once a newer one exists.
    pub fn update(&mut self, filtered: &FilteredAnalysis) {
        let mut notes = filtered.notes();
        if notes.len() > crate::prompt::MAX_NOTES {
            notes.drain(..notes.len() - crate::prompt::MAX_NOTES);
        }
        self.notes = notes;
    }

    /// The notes to send with the next T1 turn.
    pub fn notes(&self) -> &[String] {
        &self.notes
    }
}

/// The reliability window of T2: the last 20 analyses' entry counts. The
/// caller records every analysis, valid or not; "unreliable" means more than a
/// third of the entries were dropped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReliabilityWindow {
    entries: VecDeque<(usize, usize)>,
}

impl ReliabilityWindow {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one analysis: entries produced and entries dropped.
    pub fn record(&mut self, counts: DropCounts) {
        self.entries.push_back((counts.produced, counts.dropped));
        while self.entries.len() > WINDOW {
            self.entries.pop_front();
        }
    }

    /// How many analyses the window holds.
    pub fn analyses(&self) -> usize {
        self.entries.len()
    }

    /// The share of entries dropped, 0.0 when nothing was recorded.
    pub fn dropped_share(&self) -> f64 {
        let produced: usize = self.entries.iter().map(|(produced, _)| produced).sum();
        let dropped: usize = self.entries.iter().map(|(_, dropped)| dropped).sum();
        if produced == 0 {
            0.0
        } else {
            dropped as f64 / produced as f64
        }
    }

    /// More than a third of the window's entries were dropped, over at least
    /// [`MIN_ANALYSES`] analyses. For the diagnostics screen.
    pub fn unreliable(&self) -> bool {
        let produced: usize = self.entries.iter().map(|(produced, _)| produced).sum();
        let dropped: usize = self.entries.iter().map(|(_, dropped)| dropped).sum();
        self.entries.len() >= MIN_ANALYSES && dropped * 3 > produced
    }
}
