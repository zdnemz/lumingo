//! Live check for S4-11: a ten-message text chat against the real provider, on
//! the text channel with no speech model loaded. Owner-run, never in CI
//! (PROMPT_CONTRACTS section 11: live calls are manual).
//!
//! ```text
//! cargo run -p tutor-engine --example chat_live
//! ```
//!
//! The provider comes from the four `TUTOR_LLM_*` environment variables or a
//! `.env` file in the working directory, exactly as the server reads them. The
//! protocol and model name are printed; the key never is. This is a tool, not
//! library code, so errors print and exit.
//!
//! The chat runs a free topic from a small bank (the real catalog is C-05's),
//! analyses each message at the T2 cadence, and prints the end summary with
//! the error patterns. Nothing here touches a database or a speech model.
//!
//! Every message is logged with the seq the caller would store it at, and each
//! analysis batch is built from that log by seq — never by array index, which
//! would feed T2 the wrong text (the first run of this check did exactly that).

use std::error::Error;

use llm_client::{Timeouts, load_env_profile};
use tokio_util::sync::CancellationToken;
use tutor_engine::{
    AnalysisCadence, AnalysisInput, AnalysisTurn, CONVERSATION_ERROR_CAP, Chat, ChatConfig,
    ChatSummary, ChatTopic, ErrorFinding, FeedbackMode, InputMode, ReliabilityWindow, ReplyOutcome,
    SummarisedTurn, TopicBank, run_analysis, session_summary,
};

/// Ten fixed learner messages: greetings, a small mistake or two, and a
/// goodbye. They stand in for the owner's typing.
const LEARNER_LINES: [&str; 10] = [
    "Hello! I am happy to talk with you.",
    "I go to the cafe yesterday.",
    "I want a coffee and a cake, please.",
    "My friend she like tea.",
    "Thank you! How much is it?",
    "I have two brother.",
    "Yes, I like this cafe very much.",
    "Can you speak slow, please?",
    "Thank you for helping me.",
    "Goodbye! See you tomorrow.",
];

/// A three-entry bank for A1, the size the roadmap names for S4-11 until C-05
/// writes the real catalog.
const BANK: &str = r#"{
    "schema_version": "1.0",
    "version": 1,
    "levels": {
        "A1": {
            "conversations": [
                {
                    "id": "cafe",
                    "title": {"en": "At a cafe"},
                    "scenario": {"en": "You order a drink and pay."},
                    "tutor_role": "a friendly waiter",
                    "learner_role": "a customer",
                    "goals": ["Order a drink", "Say the price"]
                },
                {
                    "id": "classmate",
                    "title": {"en": "A new classmate"},
                    "scenario": {"en": "You meet a new classmate on your first day."},
                    "tutor_role": "a new classmate",
                    "learner_role": "Yourself",
                    "goals": ["Say your name", "Say where you are from"]
                },
                {
                    "id": "family",
                    "title": {"en": "Your family"},
                    "scenario": {"en": "You show a photo of your family."},
                    "tutor_role": "a friend",
                    "learner_role": "Yourself",
                    "goals": ["Name two family members"]
                }
            ]
        }
    }
}"#;

/// One learner message as the check logs it: the seq the caller would store it
/// at, the tutor message before it, the reply it got, and what T2 said about
/// it once a call covered it.
struct Logged {
    seq: i64,
    learner_text: String,
    tutor_before: String,
    tutor_reply: String,
    /// `None` while no completed call has covered this message.
    analysed: Option<Vec<ErrorFinding>>,
}

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
    let bank = TopicBank::parse(BANK)?;
    let topic = bank
        .conversation(curriculum::Level::A1, "cafe")
        .ok_or("the sample bank has no cafe topic")?
        .clone();

    let mut chat = Chat::start(ChatConfig {
        model: model.clone(),
        level: curriculum::Level::A1,
        mode: FeedbackMode::Accuracy,
        first_language: "Indonesian".to_owned(),
        topic: ChatTopic::Bank(topic),
    })?;
    let cancel = CancellationToken::new();

    println!("== live text chat: {protocol}, model {model} ==");
    println!("== 10 messages, text channel, no speech model ==");

    // The capability probe, as the app runs it for a new profile: it finds the
    // structured-output ladder level this provider supports, which the T2
    // calls then start from. Without it a gateway that ignores the native
    // schema (the owner's does) fails every structured call at level 1.
    let caps = client.probe(cancel.clone()).await;
    println!(
        "probe: auth {}, stream {}, structured level {:?}, first token {:?} ms",
        caps.auth_ok, caps.stream_ok, caps.structured_level, caps.ttft_ms
    );
    if !caps.auth_ok {
        eprintln!("the provider did not accept the key; stopping the live check");
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

    let mut emit = |event: tutor_engine::UiEvent| {
        if let tutor_engine::UiEvent::TutorTextDelta { delta } = event {
            print!("{delta}");
        }
    };
    let opening = chat.open(&client, &cancel, &mut emit).await?;
    println!(
        "\nopening: {:?}, {} sentence(s)",
        opening.report.outcome, opening.report.sentences
    );
    if !matches!(opening.report.outcome, ReplyOutcome::Normal) {
        eprintln!("the opening reply did not succeed; stopping the live check");
        return Ok(());
    }

    let mut cadence = AnalysisCadence::new();
    let mut window = ReliabilityWindow::new();
    let mut replies_ok = 0u32;
    let mut analyses_ok = 0u32;
    let mut analyses_failed = 0u32;
    let mut dropped_total = 0usize;
    let mut tutor_before = opening.report.text.clone();
    let mut log: Vec<Logged> = Vec::new();

    for (index, line) in LEARNER_LINES.iter().enumerate() {
        println!("\nlearner> {line}");
        print!("tutor> ");
        let turn = chat.say(&client, line, &cancel, &mut emit).await?;
        println!();
        if matches!(turn.report.outcome, ReplyOutcome::Normal) {
            replies_ok += 1;
        }
        println!(
            "turn {}/{}: T1 {:?}, {} sentence(s)",
            index + 1,
            LEARNER_LINES.len(),
            turn.report.outcome,
            turn.report.sentences
        );
        // The caller would store the message at this seq; the log keeps the
        // text the analysis must see, keyed by that seq.
        let seq = turn.learner_seq.unwrap_or((index + 1) as i64);
        log.push(Logged {
            seq,
            learner_text: (*line).to_owned(),
            tutor_before: tutor_before.clone(),
            tutor_reply: turn.report.text.clone(),
            analysed: None,
        });
        tutor_before = turn.report.text.clone();

        // T2: the analysis of this message, at the session's cadence. After a
        // failure the cadence re-carries every pending message, so a call can
        // cover more than the current one.
        cadence.enqueue(seq);
        let due = cadence.due();
        if due.is_empty() {
            println!(
                "         T2: batched, {} turn(s) waiting",
                cadence.pending()
            );
            continue;
        }
        let input = AnalysisInput::free(
            curriculum::Level::A1,
            InputMode::Text,
            "Indonesian",
            log.iter()
                .filter(|entry| due.contains(&entry.seq))
                .map(|entry| AnalysisTurn {
                    turn_seq: entry.seq,
                    tutor_before: entry.tutor_before.clone(),
                    learner_text: entry.learner_text.clone(),
                    tutor_reply: entry.tutor_reply.clone(),
                })
                .collect(),
        );
        match run_analysis(&client, &input, CONVERSATION_ERROR_CAP, &cancel).await {
            Ok(outcome) => {
                analyses_ok += 1;
                dropped_total += outcome.filtered.counts.dropped;
                window.record(outcome.filtered.counts);
                chat.apply_analysis(&outcome.filtered);
                cadence.mark_analysed(&due);
                // Every message the call carried is analysed, whatever the
                // filter kept of it.
                for entry in log.iter_mut().filter(|entry| due.contains(&entry.seq)) {
                    entry.analysed = Some(
                        outcome
                            .filtered
                            .turns
                            .iter()
                            .find(|t| t.turn_seq == entry.seq)
                            .map(|t| t.errors.clone())
                            .unwrap_or_default(),
                    );
                }
                let current_errors = log
                    .iter()
                    .find(|entry| entry.seq == seq)
                    .and_then(|entry| entry.analysed.as_ref())
                    .map(Vec::len)
                    .unwrap_or_default();
                println!(
                    "         T2: valid at ladder level {}, covering {} turn(s), \
                     {} error(s) on this turn, {} dropped",
                    outcome.ladder_level,
                    due.len(),
                    current_errors,
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

    // Session end: flush whatever the cadence still holds (normally nothing).
    let left = cadence.flush();
    if !left.is_empty() {
        println!("\nflush: {} turn(s) left at session end", left.len());
        let input = AnalysisInput::free(
            curriculum::Level::A1,
            InputMode::Text,
            "Indonesian",
            log.iter()
                .filter(|entry| left.contains(&entry.seq))
                .map(|entry| AnalysisTurn {
                    turn_seq: entry.seq,
                    tutor_before: entry.tutor_before.clone(),
                    learner_text: entry.learner_text.clone(),
                    tutor_reply: String::new(),
                })
                .collect(),
        );
        match run_analysis(&client, &input, CONVERSATION_ERROR_CAP, &cancel).await {
            Ok(outcome) => {
                analyses_ok += 1;
                dropped_total += outcome.filtered.counts.dropped;
                window.record(outcome.filtered.counts);
                for entry in log.iter_mut().filter(|entry| left.contains(&entry.seq)) {
                    entry.analysed = Some(
                        outcome
                            .filtered
                            .turns
                            .iter()
                            .find(|t| t.turn_seq == entry.seq)
                            .map(|t| t.errors.clone())
                            .unwrap_or_default(),
                    );
                }
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

    let summarised: Vec<SummarisedTurn> = log
        .iter()
        .map(|entry| SummarisedTurn {
            seq: entry.seq,
            analysed: entry.analysed.is_some(),
            errors: entry.analysed.clone().unwrap_or_default(),
        })
        .collect();
    let summary: ChatSummary = session_summary(&summarised, window.unreliable());
    println!();
    println!("== summary ==");
    println!("T1 replies: {replies_ok}/{} normal", LEARNER_LINES.len());
    println!(
        "T2 analyses: {analyses_ok} valid, {analyses_failed} failed, {dropped_total} entries dropped"
    );
    println!(
        "reliability: {} analyse(s) in the window, {:.0}% dropped, unreliable: {}",
        window.analyses(),
        window.dropped_share() * 100.0,
        window.unreliable()
    );
    println!(
        "chat: {} learner turn(s), {} unanalysed",
        summary.learner_turns,
        summary.unanalysed_turns.len()
    );
    for pattern in &summary.top_errors {
        println!(
            "  {} x{}: \"{}\" -> \"{}\"",
            pattern.category, pattern.count, pattern.quote, pattern.correction
        );
    }
    Ok(())
}
