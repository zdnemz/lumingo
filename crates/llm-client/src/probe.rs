//! The capability probe, also called the connection test
//! (`docs/PROMPT_CONTRACTS.md` section 4). It runs when a profile is created or
//! changed and on demand, makes a handful of small requests, and fills the
//! capabilities object.

use std::time::Instant;

use futures_util::StreamExt;
use tokio_util::sync::CancellationToken;

use crate::adapter::{CompletionRequest, Format};
use crate::error::{InvalidOutput, LlmError};
use crate::ladder::{CallSpec, ProviderClient};
use crate::schema::Contract;
use crate::types::{Capabilities, ChatMessage, LadderLevel, RateLimit, StreamEvent, TextRequest};
use crate::validate::{CompiledSchema, contract_schema, probe_schema};

/// Step 1 asks for a few tokens. Models that think before they answer may use all
/// of them for thinking; the call still proves that the key and model work.
const STEP1_MAX_TOKENS: u32 = 16;
/// Step 2 leaves room for a model that thinks first.
const STEP2_MAX_TOKENS: u32 = 256;
const STEP3_MAX_TOKENS: u32 = 512;
const STEP4_MAX_TOKENS: u32 = 2048;
/// Throughput over a shorter stretch than this is noise.
const MIN_MEASURE_SECONDS: f64 = 0.05;

const STEP3_USER: &str = "Return a JSON object with title set to \"A short test\", word_count set to 3 and is_ok set to true.";
const STEP4_SYSTEM: &str = "You fill in a JSON structure for a software test. Use short, plain English. Put exactly one entry in every list. Return only JSON that matches the schema.";
const STEP4_USER: &str = "Write one small, valid example.";

/// A level or contract failed in a way that says something about the provider's
/// support for it, as opposed to the provider being unreachable or refusing the key.
fn is_support_failure(error: &LlmError) -> bool {
    matches!(
        error,
        LlmError::Rejected { .. }
            | LlmError::Protocol(_)
            | LlmError::Refusal
            | LlmError::InvalidOutput(_)
            | LlmError::Stream { .. }
    )
}

/// Failures that say nothing about a probe step: the learner stopped it, the
/// credentials or the configuration are wrong, or the provider is rate limiting.
fn stops_the_probe(error: &LlmError) -> bool {
    matches!(
        error,
        LlmError::Cancelled
            | LlmError::Auth { .. }
            | LlmError::RateLimited { .. }
            | LlmError::HostNotAllowed { .. }
            | LlmError::InsecureScheme { .. }
            | LlmError::InvalidRequest(_)
    )
}

fn merge_rate_limit(into: &mut RateLimit, seen: RateLimit) {
    into.rpm = into.rpm.or(seen.rpm);
    into.rpd = into.rpd.or(seen.rpd);
}

impl ProviderClient {
    /// Runs the five probe steps and stores the result in the shared capabilities.
    ///
    /// A failure of step 1 (bad key, unknown model, provider unreachable) is
    /// returned as the error, because the learner needs the reason. A provider that
    /// works but supports no structured level gives `structured_level: None`.
    pub async fn probe(&self, cancel: &CancellationToken) -> Result<Capabilities, LlmError> {
        // The adapter writes the quirks it learns into the shared capabilities, so
        // the snapshot is taken after step 1 and 2 have run.
        let mut rate_limit = RateLimit::default();

        // Step 1: a minimal non-streaming call. Learns auth, the token limit
        // parameter and temperature support through the adapter's quirk handling.
        let messages = [ChatMessage::user("Reply with the word ok.")];
        let step1 = CompletionRequest {
            system: "",
            messages: &messages,
            max_tokens: STEP1_MAX_TOKENS,
            temperature: Some(0.0),
            format: Format::Plain,
        };
        match self.adapter.complete(&step1, cancel).await {
            Ok(completion) => merge_rate_limit(&mut rate_limit, completion.rate_limit),
            Err(error) => {
                if matches!(error, LlmError::Auth { .. }) {
                    self.caps.update(|caps| caps.auth_ok = false);
                }
                return Err(error);
            }
        }

        // Step 2: a streaming call for one short sentence.
        let stream_result = self.probe_stream(cancel).await?;

        // Step 3: the structured ladder with the three-field test schema.
        let level = self.probe_level(cancel, &mut rate_limit).await?;

        // Step 4: one request per contract at the chosen level.
        let mut contracts_ok = Vec::new();
        if let Some(level) = level {
            for contract in Contract::ALL {
                match self.probe_contract(level, contract, cancel).await {
                    Ok(completion_rate) => {
                        merge_rate_limit(&mut rate_limit, completion_rate);
                        contracts_ok.push(contract.name().to_owned());
                    }
                    Err(error) if is_support_failure(&error) => {
                        tracing::debug!(target: "llm_client::probe", contract = contract.name(), "contract probe failed");
                    }
                    Err(error) => return Err(error),
                }
            }
        }

        let mut result = self.caps.snapshot();
        result.probe_version = Capabilities::PROBE_VERSION;
        result.auth_ok = true;
        result.stream_ok = stream_result.stream_ok;
        result.ttft_ms = stream_result.ttft_ms;
        result.tokens_per_second = stream_result.tokens_per_second;
        if let Some(usage_seen) = stream_result.usage_seen {
            result.supports_usage_in_stream = usage_seen;
        }
        result.structured_level = level;
        result.contracts_ok = contracts_ok;
        result.rate_limit = rate_limit;
        self.caps.replace(result.clone());
        self.clear_validity();
        Ok(result)
    }

    async fn probe_stream(&self, cancel: &CancellationToken) -> Result<StreamProbe, LlmError> {
        let request = TextRequest::new(
            "You are a helpful assistant.",
            vec![ChatMessage::user(
                "Write one short sentence about the weather.",
            )],
            STEP2_MAX_TOKENS,
        );
        let mut probe = StreamProbe::default();
        let mut stream = match self.adapter.stream_text(&request, cancel).await {
            Ok(stream) => stream,
            // Step 1 proved that the key and model work, so a failure to stream, or a
            // first token that comes too late, is a finding about streaming.
            Err(error) if !stops_the_probe(&error) => return Ok(probe),
            Err(error) => return Err(error),
        };

        let mut chunks: u32 = 0;
        let mut first_chunk_at: Option<Instant> = None;
        let mut finished = None;
        while let Some(item) = stream.next().await {
            match item {
                Ok(StreamEvent::Delta(_)) => {
                    chunks += 1;
                    first_chunk_at.get_or_insert_with(Instant::now);
                }
                Ok(StreamEvent::Finished(summary)) => finished = Some(summary),
                Err(LlmError::Cancelled) => return Err(LlmError::Cancelled),
                // A stream that fails or is too slow is a finding, not a failed probe.
                Err(_) => return Ok(probe),
            }
        }
        let Some(summary) = finished else {
            return Ok(probe);
        };
        probe.stream_ok = chunks > 0;
        probe.ttft_ms = summary
            .time_to_first_token
            .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
        probe.usage_seen = Some(summary.usage.is_some());
        // Tokens per second after the first content token. The provider's own count is
        // used when it reports one; otherwise the number of stream chunks, which is
        // close to the token count on `openai_chat` servers and lower on others.
        // A model that thinks first has its thinking tokens in the provider's count, so
        // treat the figure as an estimate of how fast text arrives, not a benchmark.
        if let Some(first) = first_chunk_at {
            let seconds = first.elapsed().as_secs_f64();
            let tokens = summary
                .usage
                .and_then(|u| u.output_tokens)
                .map_or(f64::from(chunks), f64::from);
            if seconds >= MIN_MEASURE_SECONDS && tokens >= 2.0 {
                probe.tokens_per_second = Some(((tokens - 1.0) / seconds * 10.0).round() / 10.0);
            }
        }
        Ok(probe)
    }

    /// Step 3: the first level, from 1 up, at which the test schema comes back
    /// valid without a repair.
    async fn probe_level(
        &self,
        cancel: &CancellationToken,
        rate_limit: &mut RateLimit,
    ) -> Result<Option<LadderLevel>, LlmError> {
        let schema = probe_schema();
        let messages = [ChatMessage::user(STEP3_USER)];
        let spec = CallSpec {
            system: "You return JSON.",
            messages: &messages,
            max_tokens: STEP3_MAX_TOKENS,
            temperature: Some(0.0),
        };
        let mut level = Some(LadderLevel::NativeSchema);
        while let Some(current) = level {
            match self.probe_once(current, &spec, schema, cancel).await {
                Ok(seen) => {
                    merge_rate_limit(rate_limit, seen);
                    return Ok(Some(current));
                }
                Err(error) if is_support_failure(&error) => {
                    level = current.next(self.adapter.protocol());
                }
                Err(error) => return Err(error),
            }
        }
        Ok(None)
    }

    async fn probe_contract(
        &self,
        level: LadderLevel,
        contract: Contract,
        cancel: &CancellationToken,
    ) -> Result<RateLimit, LlmError> {
        let messages = [ChatMessage::user(STEP4_USER)];
        let spec = CallSpec {
            system: STEP4_SYSTEM,
            messages: &messages,
            max_tokens: STEP4_MAX_TOKENS,
            temperature: Some(0.0),
        };
        self.probe_once(level, &spec, contract_schema(contract), cancel)
            .await
    }

    /// One request at `level`, validated, with no repair: the probe reports what the
    /// provider does on its own.
    async fn probe_once(
        &self,
        level: LadderLevel,
        spec: &CallSpec<'_>,
        schema: &CompiledSchema,
        cancel: &CancellationToken,
    ) -> Result<RateLimit, LlmError> {
        let completion = self
            .call_level(level, spec, spec.messages, spec.max_tokens, schema, cancel)
            .await?;
        match schema.check_text(&completion.text) {
            Ok(_) => Ok(completion.rate_limit),
            Err(invalid) => Err(LlmError::InvalidOutput(InvalidOutput {
                reason: invalid.reason,
                paths: invalid.violations.into_iter().map(|v| v.path).collect(),
                ladder_level: level,
                repaired: false,
            })),
        }
    }
}

#[derive(Debug, Default)]
struct StreamProbe {
    stream_ok: bool,
    ttft_ms: Option<u64>,
    tokens_per_second: Option<f64>,
    usage_seen: Option<bool>,
}

#[cfg(test)]
mod tests;
