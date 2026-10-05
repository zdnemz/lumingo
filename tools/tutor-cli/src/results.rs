//! The result file of a scripted run: JSON lines.
//!
//! The first line describes the run, then one line per turn with the latency
//! parts, then one summary line with p50 and p95. Nothing in it is learner text,
//! a prompt, a reply or a key: the file is meant to be kept next to a benchmark.

use std::path::Path;

use anyhow::{Context, Result};
use app_core::voice::{LatencyRecord, LatencySummary, TurnLatency, VoiceStats};
use serde::Serialize;

use crate::observer::{TurnRecord, outcome_name};

/// What the run was made of. A number from a run with fake engines says nothing
/// about real speech, so the file says which engines produced it.
#[derive(Debug, Clone, Serialize)]
pub struct RunInfo {
    pub tool: &'static str,
    pub version: &'static str,
    /// `text` (typed input), `audio` (recordings fed through the VAD and the
    /// recogniser) or `mixed`.
    pub input: String,
    pub backend: String,
    pub scenario_unit: Option<String>,
    pub scenario_activity: Option<String>,
    pub feedback_mode: String,
    pub provider_host: String,
    pub provider_model: String,
    pub provider_protocol: String,
    pub turns_planned: usize,
    pub end_silence_ms: u32,
    pub stt: Option<EngineLine>,
    pub tts: Option<EngineLine>,
    /// A warning that belongs with the numbers, for example that the engines were fakes.
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EngineLine {
    pub id: String,
    pub version: String,
    pub model_checksum: Option<String>,
}

impl From<speech::EngineInfo> for EngineLine {
    fn from(info: speech::EngineInfo) -> Self {
        Self {
            id: info.id,
            version: info.version,
            model_checksum: info.model_checksum,
        }
    }
}

#[derive(Serialize)]
struct TurnLine<'a> {
    kind: &'static str,
    input: &'a str,
    outcome: &'a str,
    #[serde(flatten)]
    latency: LatencyRecord,
}

#[derive(Serialize)]
struct SummaryLine<'a> {
    kind: &'static str,
    /// How the run ended: `completed`, `provider_unavailable`, `interrupted` or `failed`.
    ended: &'a str,
    turns_planned: usize,
    turns_ended: usize,
    latency: &'a LatencySummary,
    frames_dropped: u64,
    inbox_dropped: u64,
    utterances_dropped: u64,
    recorder_dropped: u64,
}

#[derive(Serialize)]
struct RunLine<'a> {
    kind: &'static str,
    #[serde(flatten)]
    run: &'a RunInfo,
}

/// The turns of a run as latency values: a turn that produced no latency (it
/// was stopped, or the provider failed) is reported with every part missing.
pub fn latencies(records: &[TurnRecord]) -> Vec<TurnLatency> {
    records.iter().filter_map(|r| r.latency).collect()
}

/// The text of the file.
pub fn render(
    run: &RunInfo,
    records: &[TurnRecord],
    ended: &str,
    stats: &VoiceStats,
) -> Result<String> {
    let mut out = String::new();
    push_line(&mut out, &RunLine { kind: "run", run })?;
    for record in records {
        let latency = record.latency.unwrap_or(TurnLatency {
            turn: record.turn,
            parts: app_core::voice::LatencyParts::default(),
        });
        push_line(
            &mut out,
            &TurnLine {
                kind: "turn",
                input: record.input,
                outcome: outcome_name(record.outcome),
                latency: latency.record(),
            },
        )?;
    }
    let summary = LatencySummary::of(&latencies(records));
    push_line(
        &mut out,
        &SummaryLine {
            kind: "summary",
            ended,
            turns_planned: run.turns_planned,
            turns_ended: records.len(),
            latency: &summary,
            frames_dropped: stats.frames_dropped,
            inbox_dropped: stats.inbox_dropped,
            utterances_dropped: stats.utterances_dropped,
            recorder_dropped: stats.recorder_dropped,
        },
    )?;
    Ok(out)
}

fn push_line<T: Serialize>(out: &mut String, value: &T) -> Result<()> {
    out.push_str(&serde_json::to_string(value).context("a result line could not be written")?);
    out.push('\n');
    Ok(())
}

pub async fn write(path: &Path, text: String) -> Result<()> {
    tokio::fs::write(path, text)
        .await
        .with_context(|| format!("cannot write the result file {}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use app_core::voice::{LatencyParts, TurnOutcome};

    use super::*;

    fn run() -> RunInfo {
        RunInfo {
            tool: "tutor-cli",
            version: "0.0.0",
            input: "text".to_owned(),
            backend: "none".to_owned(),
            scenario_unit: Some("a1-u01".to_owned()),
            scenario_activity: Some("a11-roleplay-classmate".to_owned()),
            feedback_mode: "fluency".to_owned(),
            provider_host: "api.example.test".to_owned(),
            provider_model: "m".to_owned(),
            provider_protocol: "openai_chat".to_owned(),
            turns_planned: 2,
            end_silence_ms: 600,
            stt: None,
            tts: Some(EngineLine {
                id: "fake-tts".to_owned(),
                version: "test".to_owned(),
                model_checksum: None,
            }),
            warning: Some("fake engines".to_owned()),
        }
    }

    fn record(turn: u64, llm_ms: u64, outcome: TurnOutcome) -> TurnRecord {
        TurnRecord {
            turn,
            input: "text",
            outcome,
            latency: Some(TurnLatency {
                turn,
                parts: LatencyParts {
                    llm_first_sentence: Some(Duration::from_millis(llm_ms)),
                    ..LatencyParts::default()
                },
            }),
        }
    }

    #[test]
    fn the_file_has_a_run_line_a_line_per_turn_and_a_summary() {
        let records = [
            record(1, 800, TurnOutcome::Replied),
            record(2, 1_000, TurnOutcome::Replied),
        ];
        let text = render(&run(), &records, "completed", &VoiceStats::default()).unwrap();
        let lines: Vec<serde_json::Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0]["kind"], "run");
        assert_eq!(lines[0]["scenario_unit"], "a1-u01");
        assert_eq!(lines[0]["warning"], "fake engines");
        assert_eq!(lines[1]["kind"], "turn");
        assert_eq!(lines[1]["llm_first_sentence_ms"], 800.0);
        assert_eq!(lines[1]["sum_ms"], 800.0);
        assert_eq!(lines[1]["complete"], false);
        assert!(lines[1]["endpointing_wait_ms"].is_null());
        assert_eq!(lines[2]["outcome"], "replied");
        assert_eq!(lines[3]["kind"], "summary");
        assert_eq!(lines[3]["ended"], "completed");
        assert_eq!(lines[3]["turns_ended"], 2);
        assert_eq!(lines[3]["latency"]["llm_first_sentence"]["p50_ms"], 800.0);
        assert_eq!(lines[3]["latency"]["llm_first_sentence"]["p95_ms"], 1_000.0);
        // No complete turn, so no end-to-end figure.
        assert!(lines[3]["latency"]["sum"]["p50_ms"].is_null());
    }

    #[test]
    fn a_turn_without_latency_is_still_listed_with_its_outcome() {
        let records = [TurnRecord {
            turn: 1,
            input: "opening",
            outcome: TurnOutcome::ProviderUnavailable,
            latency: None,
        }];
        let text = render(
            &run(),
            &records,
            "provider_unavailable",
            &VoiceStats::default(),
        )
        .unwrap();
        let turn: serde_json::Value = serde_json::from_str(text.lines().nth(1).unwrap()).unwrap();
        assert_eq!(turn["outcome"], "provider_unavailable");
        assert_eq!(turn["sum_ms"], 0.0);
        assert_eq!(turn["complete"], false);
    }
}
