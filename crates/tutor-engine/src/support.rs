//! Small pieces shared by the services: the clock, call logging, text folding
//! for the verbatim-quote rule and the guards on model-written prose.

use std::sync::Arc;
use std::time::Duration;

use assessment_engine::Level;
use llm_client::{LlmError, StructuredOutput};
use storage::{Database, LlmCallType, LlmOutcome, NewLlmCall, Timestamp};

/// Where "now" comes from. The services never read the system clock directly, so
/// tests are deterministic and the storage crate keeps its promise that every
/// timestamp comes from the caller.
pub type Clock = Arc<dyn Fn() -> Timestamp + Send + Sync>;

/// The real clock.
pub fn system_clock() -> Clock {
    Arc::new(Timestamp::now)
}

pub(crate) fn storage_level(level: Level) -> storage::Level {
    match level {
        Level::A1 => storage::Level::A1,
        Level::A2 => storage::Level::A2,
        Level::B1 => storage::Level::B1,
        Level::B2 => storage::Level::B2,
        Level::C1 => storage::Level::C1,
        Level::C2 => storage::Level::C2,
    }
}

pub(crate) fn curriculum_level(level: Level) -> curriculum::Level {
    match level {
        Level::A1 => curriculum::Level::A1,
        Level::A2 => curriculum::Level::A2,
        Level::B1 => curriculum::Level::B1,
        Level::B2 => curriculum::Level::B2,
        Level::C1 => curriculum::Level::C1,
        Level::C2 => curriculum::Level::C2,
    }
}

/// Lower-cases and collapses whitespace, the form in which a quote is compared
/// with the text it claims to come from.
pub(crate) fn fold(text: &str) -> String {
    text.split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

/// True when `quote` is non-empty and occurs in `text` after folding both.
pub(crate) fn quote_in(text_folded: &str, quote: &str) -> bool {
    let quote = fold(quote);
    !quote.is_empty() && text_folded.contains(&quote)
}

/// Whether model-written prose names a CEFR level. A model never sets or states
/// a level, so prose that does is not used. The check is deliberately broad: a
/// false positive costs one note, a false negative costs a rule of the project.
pub(crate) fn names_a_level(text: &str) -> bool {
    if text.to_lowercase().contains("cefr") {
        return true;
    }
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|token| {
            let bytes = token.as_bytes();
            bytes.len() == 2
                && matches!(bytes[0].to_ascii_uppercase(), b'A'..=b'C')
                && matches!(bytes[1], b'1' | b'2')
        })
}

/// Whether prose states a percentage, which the rubric contract forbids.
pub(crate) fn states_a_percentage(text: &str) -> bool {
    text.contains('%') || text.to_lowercase().contains("percent")
}

/// Cuts text to at most `max_words` words, for the limits a schema cannot express.
pub(crate) fn truncate_words(text: &str, max_words: usize) -> String {
    text.split_whitespace()
        .take(max_words)
        .collect::<Vec<_>>()
        .join(" ")
}

/// A short single line from learner input: control characters and line breaks
/// become spaces, runs of spaces collapse, and the result is cut to `max_chars`.
/// Used for a typed topic, which ends up inside the system prompt.
pub(crate) fn single_line(text: &str, max_chars: usize) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let collapsed = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(max_chars).collect()
}

/// The outcome and HTTP status to log for a failed call.
fn failure(error: &LlmError) -> (LlmOutcome, Option<i64>) {
    match error {
        LlmError::Timeout(_) => (LlmOutcome::Timeout, None),
        LlmError::RateLimited { .. } => (LlmOutcome::RateLimited, Some(429)),
        LlmError::Refusal => (LlmOutcome::Refused, None),
        LlmError::Cancelled => (LlmOutcome::Cancelled, None),
        LlmError::InvalidOutput(_) => (LlmOutcome::InvalidOutput, None),
        LlmError::Auth { status } => (LlmOutcome::Error, Some(i64::from(*status))),
        LlmError::Server { status, .. } | LlmError::Rejected { status, .. } => {
            (LlmOutcome::Error, Some(i64::from(*status)))
        }
        _ => (LlmOutcome::Error, None),
    }
}

/// What one call looked like from the outside, for `llm_calls`. The table holds
/// no learner text, no prompt and no reply, so nothing here carries any.
pub(crate) struct CallLog<'a> {
    pub db: &'a Database,
    pub provider_profile_id: Option<i64>,
    pub call_type: LlmCallType,
    pub model: &'a str,
    pub started_at: Timestamp,
    pub elapsed: Duration,
}

impl CallLog<'_> {
    /// Records a structured call. A failure to log is reported and swallowed:
    /// diagnostics must never turn a good answer into an error.
    pub(crate) async fn structured(&self, result: &Result<StructuredOutput, LlmError>) {
        let (outcome, http_status, ladder_level, usage) = match result {
            Ok(out) => (
                if out.repaired {
                    LlmOutcome::Repaired
                } else {
                    LlmOutcome::Ok
                },
                None,
                Some(i64::from(out.ladder_level.as_u8())),
                out.usage,
            ),
            Err(error) => {
                let (outcome, status) = failure(error);
                let ladder = match error {
                    LlmError::InvalidOutput(invalid) => {
                        Some(i64::from(invalid.ladder_level.as_u8()))
                    }
                    _ => None,
                };
                (outcome, status, ladder, None)
            }
        };
        self.write(outcome, http_status, ladder_level, None, usage)
            .await;
    }

    /// Records a streamed call that ended with `outcome`.
    pub(crate) async fn streamed(
        &self,
        outcome: LlmOutcome,
        ttft: Option<Duration>,
        usage: Option<llm_client::Usage>,
    ) {
        self.write(outcome, None, None, ttft, usage).await;
    }

    /// The outcome to log for a failed call of any kind.
    pub(crate) fn outcome_of(error: &LlmError) -> LlmOutcome {
        failure(error).0
    }

    async fn write(
        &self,
        outcome: LlmOutcome,
        http_status: Option<i64>,
        ladder_level: Option<i64>,
        ttft: Option<Duration>,
        usage: Option<llm_client::Usage>,
    ) {
        let millis = |d: Duration| i64::try_from(d.as_millis()).unwrap_or(i64::MAX);
        let row = NewLlmCall {
            provider_profile_id: self.provider_profile_id,
            call_type: self.call_type,
            model: self.model.to_owned(),
            ladder_level,
            ttft_ms: ttft.map(millis),
            total_ms: Some(millis(self.elapsed)),
            input_tokens: usage.and_then(|u| u.input_tokens).map(i64::from),
            output_tokens: usage.and_then(|u| u.output_tokens).map(i64::from),
            http_status,
            outcome,
            started_at: self.started_at,
        };
        if let Err(error) = self.db.diagnostics().record_llm_call(&row).await {
            tracing::warn!(%error, "could not record an LLM call");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folding_ignores_case_and_spacing() {
        assert_eq!(fold("  I   Go\tTo\nSchool "), "i go to school");
    }

    #[test]
    fn a_quote_must_be_non_empty_and_inside_the_text() {
        let text = fold("Yesterday I go to the market.");
        assert!(quote_in(&text, "I  GO to"));
        assert!(!quote_in(&text, "I went"));
        assert!(!quote_in(&text, ""));
        assert!(!quote_in(&text, "   "));
    }

    #[test]
    fn prose_that_names_a_level_is_recognised() {
        for text in [
            "Practise at A2 level",
            "your English is b1",
            "a CEFR thing",
            "C2.",
        ] {
            assert!(names_a_level(text), "{text}");
        }
        for text in [
            "Practise a and an",
            "Use the past simple",
            "Room 12 is open",
            "A3 paper",
            "ab1",
        ] {
            assert!(!names_a_level(text), "{text}");
        }
    }

    #[test]
    fn percentages_are_recognised() {
        assert!(states_a_percentage("You scored 80%"));
        assert!(states_a_percentage("eighty percent right"));
        assert!(!states_a_percentage("Good work"));
    }

    #[test]
    fn words_are_truncated_on_word_boundaries() {
        assert_eq!(truncate_words("a b  c d", 3), "a b c");
        assert_eq!(truncate_words("a b", 5), "a b");
    }

    #[test]
    fn a_typed_topic_becomes_one_short_line() {
        assert_eq!(
            single_line("my\ndog \u{7} and\r\n  cat", 80),
            "my dog and cat"
        );
        assert_eq!(single_line("abcdef", 3), "abc");
    }
}
