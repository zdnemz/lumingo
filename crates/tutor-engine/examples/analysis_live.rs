//! Live smoke test for S4-06: 20 fixed learner turns through T1 (streamed
//! reply) and T2 (structured analysis), against the real provider. Reports
//! validity, time to first token, and dropped entries. Owner-run, never in CI
//! (PROMPT_CONTRACTS section 11: live calls are manual).
//!
//! ```text
//! cargo run -p tutor-engine --example analysis_live
//! ```
//!
//! The capability probe runs first, as the app runs it when a profile is
//! created: it picks the structured-output ladder level this provider can do
//! and caches it for the T2 calls. Without it, structured calls start at level
//! 1 (native schema), which third-party Anthropic-compatible endpoints may not
//! implement (PROMPT_CONTRACTS section 3).
//!
//! The provider comes from the four `TUTOR_LLM_*` environment variables or a
//! `.env` file in the working directory, exactly as the server reads them. The
//! protocol and model name are printed; the key never is. This is a tool, not
//! library code, so errors print and exit.

use std::error::Error;
use std::time::{Duration, Instant};

use llm_client::{Timeouts, load_env_profile};
use tokio_util::sync::CancellationToken;
use tutor_engine::{
    AnalysisCadence, AnalysisInput, AnalysisTurn, CONVERSATION_ERROR_CAP, Channel, Event,
    FeedbackMode, InputMode, ReliabilityWindow, Session, SessionKind, TutorContext, UiEvent,
    opening_request, run_analysis, run_reply, text_request,
};

const EXAMPLE: &str = include_str!("../../../curriculum/examples/a1-u01.example.json");

/// Twenty fixed learner lines for the example unit's roleplay: greetings,
/// introductions, and a few controlled mistakes for the analysis to find.
const LEARNER_LINES: [&str; 20] = [
    "Hello! My name is Dewi.",
    "I am from Bandung.",
    "I go to school yesterday.",
    "Nice to meet you too!",
    "I have two brother.",
    "My hobby is reading book.",
    "Yes, I like music very much.",
    "She go to my school.",
    "I am study English every day.",
    "What is your favorite food?",
    "I like fried rice, it is delicious.",
    "I want to be a doctor.",
    "Yesterday I watch a movie with my family.",
    "My mother cook rendang.",
    "I am happy today because the weather is good.",
    "Can you speak slow, please?",
    "I not understand that word.",
    "Thank you for helping me.",
    "See you tomorrow!",
    "Goodbye!",
];

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let Some(config) = load_env_profile(Some(std::path::Path::new(".env")))? else {
        eprintln!(
            "No provider configured. Set TUTOR_LLM_PROTOCOL, TUTOR_LLM_BASE_URL, \
             TUTOR_LLM_MODEL and TUTOR_LLM_API_KEY in the environment or in .env."
        );
        return Ok(());
    };
    let protocol = config.protocol.name();
    let model = config.model.clone();
    let client = llm_client::LlmClient::new(config, Timeouts::default())?;
    let unit = curriculum::UnitLoader::new().load_str(EXAMPLE)?;
    let cancel = CancellationToken::new();

    let context = TutorContext::from_roleplay(
        &unit,
        Some("a11-roleplay-classmate"),
        Channel::Voice,
        Some(FeedbackMode::Accuracy),
        "Indonesian",
    )?;
    let mut session = Session::new(SessionKind::Conversation, Channel::Voice);

    println!("== live analysis smoke test: {protocol}, model {model} ==");
    println!("== 20 fixed turns, T1 then T2 each ==");

    // The capability probe, as the app runs it for a new profile: it measures
    // speed and finds the structured-output ladder level this provider
    // supports, which the T2 calls then start from.
    let caps = client.probe(cancel.clone()).await;
    println!(
        "probe: auth {}, stream {}, structured level {:?}, first token {:?} ms, tokens/s {:?}",
        caps.auth_ok,
        caps.stream_ok,
        caps.structured_level,
        caps.ttft_ms,
        caps.tokens_per_second.map(|t| (t * 10.0).round() / 10.0)
    );
    if !caps.auth_ok {
        eprintln!("the provider did not accept the key; stopping the smoke test");
        return Ok(());
    }
    if caps.structured_level.is_none() {
        eprintln!(
            "warning: no structured-output ladder level worked in the probe; T2 calls will fail"
        );
    }
    // Diagnosis switch for live checks: force a ladder level, to compare what a
    // provider does at each level when the probe's choice does not work.
    if let Ok(forced) = std::env::var("TUTOR_LLM_FORCE_LEVEL")
        && let Some(level) = forced.parse().ok().and_then(llm_client::Level::from_number)
    {
        client.set_structured_level(level);
        println!("forced structured level {}", level.number());
    }

    // The tutor opens; its reply is the first `tutor_before` of the log.
    session.apply(Event::OpeningTurn)?;
    let request = opening_request(&model, &context, &[]);
    let started = Instant::now();
    let report = run_reply(&mut session, &client, request, &cancel, true, &mut |_| {}).await?;
    let ttft = started.elapsed();
    println!(
        "opening: {:?} in {:.0} ms, {} sentence(s)",
        report.outcome,
        ttft.as_secs_f64() * 1000.0,
        report.sentences
    );
    if !matches!(report.outcome, tutor_engine::ReplyOutcome::Normal) {
        eprintln!("the opening reply did not succeed; stopping the smoke test");
        return Ok(());
    }
    let mut history = vec![llm_client::Message {
        role: llm_client::Role::Assistant,
        content: report.text.clone(),
    }];
    let mut tutor_before = report.text;

    let mut cadence = AnalysisCadence::new();
    let mut window = ReliabilityWindow::new();
    let mut replies_ok = 0u32;
    let mut analyses_ok = 0u32;
    let mut analyses_failed = 0u32;
    let mut dropped_total = 0usize;
    let mut ttft_total = Duration::ZERO;

    for (index, line) in LEARNER_LINES.iter().enumerate() {
        // T1: the tutor's reply to this learner turn.
        session.apply(Event::UtteranceEnded)?;
        session.apply(Event::TranscriptReady)?;
        let request = text_request(&model, &context, &history, line, &[], &[]);
        let mut first_text: Option<Duration> = None;
        let started = Instant::now();
        let report = run_reply(
            &mut session,
            &client,
            request,
            &cancel,
            true,
            &mut |event| {
                if let UiEvent::TutorTextDelta { .. } = event {
                    first_text.get_or_insert_with(|| started.elapsed());
                }
            },
        )
        .await?;
        let this_ttft = first_text.unwrap_or_else(|| started.elapsed());
        if matches!(report.outcome, tutor_engine::ReplyOutcome::Normal) {
            replies_ok += 1;
            ttft_total += this_ttft;
        }
        println!(
            "turn {}/{}: T1 {:?}, {:.0} ms to first token, {} sentence(s)",
            index + 1,
            LEARNER_LINES.len(),
            report.outcome,
            this_ttft.as_secs_f64() * 1000.0,
            report.sentences
        );
        history.push(llm_client::Message {
            role: llm_client::Role::User,
            content: line.to_string(),
        });
        history.push(llm_client::Message {
            role: llm_client::Role::Assistant,
            content: report.text.clone(),
        });

        // T2: the analysis of this turn, at the session's cadence.
        cadence.enqueue((index + 1) as i64);
        let due = cadence.due();
        if due.is_empty() {
            println!(
                "         T2: batched, {} turn(s) waiting",
                cadence.pending()
            );
        } else {
            let input = AnalysisInput::from_activity(
                &unit,
                Some("a11-roleplay-classmate"),
                InputMode::Voice,
                "Indonesian",
                due.iter()
                    .map(|seq| AnalysisTurn {
                        turn_seq: *seq,
                        tutor_before: tutor_before.clone(),
                        learner_text: LEARNER_LINES
                            .get((*seq - 1) as usize)
                            .unwrap_or(&"")
                            .to_string(),
                        tutor_reply: report.text.clone(),
                    })
                    .collect(),
            )?;
            match run_analysis(&client, &input, CONVERSATION_ERROR_CAP, &cancel).await {
                Ok(outcome) => {
                    analyses_ok += 1;
                    dropped_total += outcome.filtered.counts.dropped;
                    window.record(outcome.filtered.counts);
                    cadence.mark_analysed(&due);
                    println!(
                        "         T2: valid at ladder level {}, {} error(s) kept, {} dropped",
                        outcome.ladder_level,
                        outcome
                            .filtered
                            .turns
                            .iter()
                            .map(|t| t.errors.len())
                            .sum::<usize>(),
                        outcome.filtered.counts.dropped
                    );
                }
                Err(error) => {
                    analyses_failed += 1;
                    if let tutor_engine::AnalysisFailure::Provider(
                        llm_client::LlmError::RateLimited { .. },
                    ) = &error
                    {
                        cadence.note_rate_limited();
                        println!("         T2: rate limited; switching to batched cadence");
                    } else {
                        println!("         T2: {error}");
                    }
                }
            }
        }
        tutor_before = report.text;
    }

    // Session end: flush whatever the cadence still holds.
    let left = cadence.flush();
    if !left.is_empty() {
        println!("flush: {} turn(s) left at session end", left.len());
        let input = AnalysisInput::from_activity(
            &unit,
            Some("a11-roleplay-classmate"),
            InputMode::Voice,
            "Indonesian",
            left.iter()
                .map(|seq| AnalysisTurn {
                    turn_seq: *seq,
                    tutor_before: tutor_before.clone(),
                    learner_text: LEARNER_LINES
                        .get((*seq - 1) as usize)
                        .unwrap_or(&"")
                        .to_string(),
                    tutor_reply: String::new(),
                })
                .collect(),
        )?;
        match run_analysis(&client, &input, CONVERSATION_ERROR_CAP, &cancel).await {
            Ok(outcome) => {
                analyses_ok += 1;
                dropped_total += outcome.filtered.counts.dropped;
                window.record(outcome.filtered.counts);
                cadence.mark_analysed(&left);
                println!(
                    "         T2: valid at ladder level {}, {} dropped",
                    outcome.ladder_level, outcome.filtered.counts.dropped
                );
            }
            Err(error) => {
                analyses_failed += 1;
                println!("         T2: {error}");
            }
        }
    }

    let average_ttft = if replies_ok > 0 {
        ttft_total.as_secs_f64() * 1000.0 / f64::from(replies_ok)
    } else {
        0.0
    };
    println!();
    println!("== summary ==");
    println!(
        "T1 replies: {replies_ok}/{} normal, average {average_ttft:.0} ms to first token",
        LEARNER_LINES.len()
    );
    println!(
        "T2 analyses: {analyses_ok} valid, {analyses_failed} failed, {dropped_total} entries dropped"
    );
    println!(
        "reliability: {} analyse(s) in the window, {:.0}% dropped, unreliable: {}",
        window.analyses(),
        window.dropped_share() * 100.0,
        window.unreliable()
    );
    Ok(())
}
