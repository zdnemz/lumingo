//! What `tutor-cli unit run` prints: the activity, then the result.
//!
//! Scores are task scores from 0 to 1. Nothing here names a level, and nothing
//! here is worded by a model: the lines are built from the data the library
//! returns and from the unit's own authored explanations.

use std::fmt::Write as _;

use pron_engine::{UtteranceReport, WordResult};
use tutor_engine::{
    ActivityResult, AudioLine, Body, CheckpointReport, Feedback, ItemOutcome, Presentation,
    ResultOutcome, RubricOutcome, UnscoredReason,
};

fn audio_block(lines: &[AudioLine], show_text: bool) -> String {
    if !show_text {
        return format!(
            "[audio: {} line(s), play it with the speaker]\n",
            lines.len()
        );
    }
    let mut out = String::from("[audio, shown as text because no speaker is attached]\n");
    for line in lines {
        match &line.speaker {
            Some(speaker) => {
                let _ = writeln!(out, "  {speaker}: {}", line.text);
            }
            None => {
                let _ = writeln!(out, "  {}", line.text);
            }
        }
    }
    out
}

fn numbered(options: &[String]) -> String {
    options
        .iter()
        .enumerate()
        .map(|(i, option)| format!("  {}) {option}", i + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The activity as the learner sees it.
pub fn render_presentation(shown: &Presentation, show_audio_text: bool) -> String {
    let mut out = format!(
        "\n== {} ({}) ==\n{}\n",
        shown.id,
        shown.activity_type.as_str(),
        shown.instructions.en
    );
    match &shown.body {
        Body::Mcq {
            passage,
            audio,
            stem,
            options,
        } => {
            if let Some(passage) = passage {
                let _ = writeln!(out, "{passage}\n");
            }
            if let Some(audio) = audio {
                out.push_str(&audio_block(audio, show_audio_text));
            }
            let _ = writeln!(out, "{stem}\n{}", numbered(options));
        }
        Body::GapFill { text, gaps } => {
            let _ = writeln!(out, "{text}\n({gaps} gap(s): answer with `|` between them)");
        }
        Body::Reorder { tokens } => {
            let _ = writeln!(out, "{}\n(type the words in order)", tokens.join(" / "));
        }
        Body::Match { left, right } => {
            for (i, phrase) in left.iter().enumerate() {
                let _ = writeln!(out, "  {}. {phrase}", i + 1);
            }
            let _ = writeln!(out, "meanings:\n{}", numbered(right));
            out.push_str("(one number per phrase, `-` to leave blank)\n");
        }
        Body::Dictation { audio } => {
            out.push_str(&audio_block(audio, show_audio_text));
            out.push_str("(type what you hear)\n");
        }
        Body::MinimalPairsListen { items } => {
            for (i, item) in items.iter().enumerate() {
                let _ = writeln!(
                    out,
                    "  {}. 1) {}   2) {}",
                    i + 1,
                    item.options[0],
                    item.options[1]
                );
            }
            out.push_str(&audio_block(
                &items.iter().map(|i| i.audio.clone()).collect::<Vec<_>>(),
                show_audio_text,
            ));
            out.push_str("(which word did you hear in each pair: 1 or 2)\n");
        }
        Body::MinimalPairsSay { pairs } => {
            for pair in pairs {
                let _ = writeln!(out, "  say: {} / {}   ({})", pair.a, pair.b, pair.focus);
            }
            out.push_str("(`wav: <file>` per pair, or an empty line to skip the recording)\n");
        }
        Body::ReadingSet { passage, questions } => {
            let _ = writeln!(out, "{passage}\n");
            for (i, q) in questions.iter().enumerate() {
                let _ = writeln!(out, "{}. {}\n{}", i + 1, q.stem, numbered(&q.options));
            }
            out.push_str("(one number per question, `-` to leave blank)\n");
        }
        Body::ListeningSet {
            audio,
            replays_allowed,
            questions,
        } => {
            out.push_str(&audio_block(audio, show_audio_text));
            let _ = writeln!(out, "(you may play it again {replays_allowed} time(s))");
            for (i, q) in questions.iter().enumerate() {
                let _ = writeln!(out, "{}. {}\n{}", i + 1, q.stem, numbered(&q.options));
            }
            out.push_str("(one number per question, `-` to leave blank)\n");
        }
        Body::ErrorCorrection { sentence } => {
            let _ = writeln!(out, "{sentence}\n(type the sentence again, corrected)");
        }
        Body::ReadAloud { text, .. } => {
            let _ = writeln!(
                out,
                "read aloud: {text}\n(`wav: <file>`, or an empty line to skip the recording)"
            );
        }
        Body::Shadowing { title, lines } => {
            let _ = writeln!(out, "{title}");
            out.push_str(&audio_block(lines, true));
            out.push_str("(repeat each line: `wav: <file>` per line, or an empty line)\n");
        }
        Body::Production {
            prompt,
            content_points,
            min_words,
            max_words,
        } => {
            let _ = writeln!(
                out,
                "{}\ninclude: {}\n({min_words} to {max_words} words)",
                prompt.en,
                content_points.join("; ")
            );
        }
        Body::Mediation { source_text, task } => {
            let _ = writeln!(out, "{}\n---\n{source_text}\n---", task.en);
        }
        Body::Roleplay {
            scenario,
            tutor_role,
            learner_role,
            goals,
            max_turns,
        } => {
            let _ = writeln!(
                out,
                "{}\nthe tutor is: {tutor_role}\nyou are: {learner_role}\ngoals: {}\n(up to {max_turns} turns; `/end` stops early)",
                scenario.en,
                goals.join("; ")
            );
        }
    }
    out
}

fn item_lines(feedback: &Feedback) -> String {
    let mut out = String::new();
    for item in &feedback.items {
        let what = match item.outcome {
            ItemOutcome::Correct => continue,
            ItemOutcome::Spelling => "check the spelling",
            ItemOutcome::Wrong => "not right",
            ItemOutcome::Blank => "left blank",
        };
        let given = item.given.as_deref().unwrap_or("-");
        let _ = writeln!(
            out,
            "  item {}: {what}: you gave \"{given}\", expected \"{}\"",
            item.index + 1,
            item.expected
        );
        if let Some(explanation) = &item.explanation {
            let _ = writeln!(out, "    {}", explanation.en);
        }
    }
    if let Some(explanation) = &feedback.explanation
        && feedback
            .items
            .iter()
            .any(|i| i.outcome != ItemOutcome::Correct)
    {
        let _ = writeln!(out, "  {}", explanation.en);
    }
    out
}

fn rubric_lines(outcome: &RubricOutcome) -> String {
    let mut out = String::new();
    let bands: Vec<String> = outcome
        .dimensions
        .iter()
        .map(|d| match d.band {
            Some(band) => format!("{} {band}", d.dimension.as_str()),
            None => format!("{} needs review", d.dimension.as_str()),
        })
        .collect();
    let _ = writeln!(
        out,
        "  bands (0 to 4): {}\n  {} run(s), confidence {:.2}",
        bands.join(", "),
        outcome.runs,
        outcome.confidence
    );
    if outcome.try_again() {
        out.push_str("  The response did not attempt the task: try again.\n");
    }
    if !outcome.feedback_en.is_empty() {
        let _ = writeln!(out, "  {}", outcome.feedback_en);
    }
    out
}

fn pron_lines(reports: &[UtteranceReport]) -> String {
    let mut out = String::from("  pronunciation feedback is experimental\n");
    for report in reports {
        match report.utterance_score {
            Some(score) => {
                let _ = writeln!(out, "  recording score {score:.2}");
            }
            None => out.push_str("  recording not scored\n"),
        }
        for word in &report.words {
            let WordResult::Scored { text, phonemes, .. } = word else {
                continue;
            };
            for phoneme in phonemes.iter().filter(|p| p.flagged == Some(true)) {
                let heard = phoneme
                    .heard
                    .as_ref()
                    .map_or_else(String::new, |h| format!(", heard as {}", h.label));
                let _ = writeln!(out, "    {} in \"{text}\" flagged{heard}", phoneme.symbol);
            }
        }
    }
    out
}

fn unscored_line(reason: &UnscoredReason) -> String {
    match reason {
        UnscoredReason::NoScorer => "  stored, with nothing to score: this activity is practice".to_owned(),
        UnscoredReason::RubricMissing { rubric_id } => {
            format!("  stored, not scored: the rubric {rubric_id} is not in the rubrics folder")
        }
        UnscoredReason::EngineUnavailable { why } => format!("  stored, not scored: {why}"),
        UnscoredReason::InteractionRubricMissing => {
            "  stored, not scored: the interaction rubric of this level is not in the rubrics folder"
                .to_owned()
        }
    }
}

/// The result of one activity: its score and what explains it.
pub fn render_result(result: &ActivityResult) -> String {
    let mut out = String::new();
    let headline = match (&result.outcome, result.score) {
        (ResultOutcome::Queued, _) => "waiting for a provider, stored as pending".to_owned(),
        (_, Some(score)) => match result.confidence {
            Some(confidence) if confidence < 1.0 => {
                format!("score {score:.2} (confidence {confidence:.2})")
            }
            _ => format!("score {score:.2}"),
        },
        (_, None) => "not scored".to_owned(),
    };
    let _ = writeln!(
        out,
        "{}  [{}]  {headline}",
        result.activity_id, result.skill
    );
    match &result.outcome {
        ResultOutcome::Deterministic(feedback) => out.push_str(&item_lines(feedback)),
        ResultOutcome::Rubric(outcome) => out.push_str(&rubric_lines(outcome)),
        ResultOutcome::Pron(reports) => out.push_str(&pron_lines(reports)),
        ResultOutcome::Unscored(reason) => {
            out.push_str(&unscored_line(reason));
            out.push('\n');
        }
        ResultOutcome::Queued => {}
    }
    out
}

/// The checkpoint, as numbers.
pub fn render_checkpoint(report: &CheckpointReport, status: &str) -> String {
    let mut out = String::from("\ncheckpoint\n");
    for row in &report.rows {
        let score = match (row.answered, row.score) {
            (false, _) => "not answered".to_owned(),
            (true, Some(score)) => format!("{score:.2}"),
            (true, None) => "waiting for a score".to_owned(),
        };
        let _ = writeln!(out, "  {:<28} {score}", row.activity_id);
    }
    let outcome = &report.outcome;
    let verdict = match (outcome.passed, outcome.provisional) {
        (true, false) => "passed",
        (true, true) => "passed provisionally: some items still wait for a score",
        (false, true) => "not passed yet: some items still wait for a score",
        (false, false) => "not passed",
    };
    let _ = writeln!(
        out,
        "  mean {:.2} against a pass mark of {:.2}: {verdict}\n  unit status: {status}",
        outcome.mean, report.pass_mark
    );
    out
}
