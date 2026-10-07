//! Live check for S4-05: the example unit's roleplay against the real
//! provider, on both channels. Owner-run, never in CI (PROMPT_CONTRACTS
//! section 11: live calls are manual).
//!
//! ```text
//! cargo run -p tutor-engine --example roleplay_live
//! ```
//!
//! The provider comes from the four `TUTOR_LLM_*` environment variables or a
//! `.env` file in the working directory, exactly as the server reads them. The
//! protocol and model name are printed; the key never is. This is a tool, not
//! library code, so errors print and exit.

use std::error::Error;

use llm_client::{Timeouts, load_env_profile};
use tokio_util::sync::CancellationToken;
use tutor_engine::{
    Channel, Event, FeedbackMode, Session, SessionKind, TutorContext, UiEvent, opening_request,
    run_reply, text_request,
};

const EXAMPLE: &str = include_str!("../../../curriculum/examples/a1-u01.example.json");
const LEARNER_LINE: &str = "Hello! My name is Dewi. I'm from Bandung. And you?";

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

    println!("== live roleplay check: {protocol}, model {model} ==");
    for channel in [Channel::Voice, Channel::Text] {
        println!("== {channel:?} channel, roleplay a11-roleplay-classmate ==");
        run_mode(&client, &unit, &model, channel).await?;
        println!();
    }
    Ok(())
}

/// One roleplay: the tutor's opening turn, then one learner turn, with the
/// session state machine driven exactly as the server will drive it.
async fn run_mode(
    client: &llm_client::LlmClient,
    unit: &curriculum::Unit,
    model: &str,
    channel: Channel,
) -> Result<(), Box<dyn Error>> {
    let context = TutorContext::from_roleplay(
        unit,
        Some("a11-roleplay-classmate"),
        channel,
        Some(FeedbackMode::Accuracy),
        "Indonesian",
    )?;
    let mut session = Session::new(SessionKind::Conversation, channel);
    let cancel = CancellationToken::new();

    // The tutor speaks first (T1's trigger).
    session.apply(Event::OpeningTurn)?;
    let request = opening_request(model, &context, &[]);
    let mut emit = |event: UiEvent| {
        if let UiEvent::TutorSentenceSpoken { text, .. } = event {
            println!("tutor> {text}");
        }
    };
    let report = run_reply(&mut session, client, request, &cancel, true, &mut emit).await?;
    println!(
        "opening: {:?}, {} sentence(s)",
        report.outcome, report.sentences
    );
    if !matches!(report.outcome, tutor_engine::ReplyOutcome::Normal) {
        return Ok(());
    }

    // One learner turn, then the tutor's reply to it.
    match channel {
        Channel::Voice => {
            session.apply(Event::UtteranceEnded)?;
            session.apply(Event::TranscriptReady)?;
        }
        Channel::Text => {
            session.apply(Event::TextSent)?;
        }
    }
    println!("learner> {LEARNER_LINE}");
    let history = [llm_client::Message {
        role: llm_client::Role::Assistant,
        content: report.text.clone(),
    }];
    let request = text_request(model, &context, &history, LEARNER_LINE, &[], &[]);
    let mut emit = |event: UiEvent| {
        if let UiEvent::TutorSentenceSpoken { text, .. } = event {
            println!("tutor> {text}");
        }
    };
    let report = run_reply(&mut session, client, request, &cancel, true, &mut emit).await?;
    println!(
        "turn 2: {:?}, {} sentence(s)",
        report.outcome, report.sentences
    );
    println!(
        "session: {:?}, {} turn(s) completed",
        session.phase(),
        session.turns_completed()
    );
    Ok(())
}
