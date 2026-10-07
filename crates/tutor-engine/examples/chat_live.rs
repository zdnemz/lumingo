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

use std::error::Error;

use llm_client::{Timeouts, load_env_profile};
use tokio_util::sync::CancellationToken;
use tutor_engine::{
    AnalysisCadence, AnalysisInput, AnalysisTurn, CONVERSATION_ERROR_CAP, Chat, ChatConfig,
    ChatSummary, ChatTopic, FeedbackMode, InputMode, ReliabilityWindow, ReplyOutcome, TopicBank,
    run_analysis, session_summary,
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
    // One summarised turn per learner message, for the end summary.
    let mut summarised: Vec<tutor_engine::SummarisedTurn> = Vec::new();

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

        // T2: the analysis of this message, at the session's cadence.
        let seq = turn.learner_seq.unwrap_or((index + 1) as i64);
        cadence.enqueue(seq);
        let due = cadence.due();
        let mut analysed = false;
        if !due.is_empty() {
            let input = AnalysisInput::free(
                curriculum::Level::A1,
                InputMode::Text,
                "Indonesian",
                due.iter()
                    .map(|seq| AnalysisTurn {
                        turn_seq: *seq,
                        tutor_before: tutor_before.clone(),
                        learner_text: LEARNER_LINES
                            .get((*seq - 1) as usize)
                            .unwrap_or(&"")
                            .to_string(),
                        tutor_reply: turn.report.text.clone(),
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
                    analysed = true;
                    let errors = outcome
                        .filtered
                        .turns
                        .iter()
                        .find(|t| t.turn_seq == seq)
                        .map(|t| t.errors.clone())
                        .unwrap_or_default();
                    println!(
                        "         T2: valid at ladder level {}, {} error(s) kept, {} dropped",
                        outcome.ladder_level,
                        errors.len(),
                        outcome.filtered.counts.dropped
                    );
                    summarised.push(tutor_engine::SummarisedTurn {
                        seq,
                        analysed: true,
                        errors,
                    });
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
        } else {
            println!(
                "         T2: batched, {} turn(s) waiting",
                cadence.pending()
            );
        }
        if !analysed {
            summarised.push(tutor_engine::SummarisedTurn {
                seq,
                analysed: false,
                errors: Vec::new(),
            });
        }
        tutor_before = turn.report.text.clone();
    }

    // Session end: flush whatever the cadence still holds.
    let left = cadence.flush();
    if !left.is_empty() {
        println!("\nflush: {} turn(s) left at session end", left.len());
        let input = AnalysisInput::free(
            curriculum::Level::A1,
            InputMode::Text,
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
        );
        match run_analysis(&client, &input, CONVERSATION_ERROR_CAP, &cancel).await {
            Ok(outcome) => {
                analyses_ok += 1;
                dropped_total += outcome.filtered.counts.dropped;
                window.record(outcome.filtered.counts);
                for turn in &mut summarised {
                    if left.contains(&turn.seq) {
                        turn.analysed = true;
                        turn.errors = outcome
                            .filtered
                            .turns
                            .iter()
                            .find(|t| t.turn_seq == turn.seq)
                            .map(|t| t.errors.clone())
                            .unwrap_or_default();
                    }
                }
                cadence.mark_analysed(&left);
            }
            Err(error) => {
                analyses_failed += 1;
                println!("         T2: {error}");
            }
        }
    }

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
