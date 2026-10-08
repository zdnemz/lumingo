//! Engine types to API types.
//!
//! The engines' own types derive `Serialize` at most, and the API must not hand
//! the browser a shape that a refactor of an engine would silently change, so
//! every type that crosses is copied field by field here. Matches over engine
//! enums are exhaustive: a variant added to an engine stops the build here.

use pron_engine::{UtteranceReport, WordResult};
use tutor_engine::{
    ActivityResult, AudioLine, Body, ChatSummary, DimensionStatus, DraftComparison, DraftFeedback,
    DraftStatus, EvidenceStatus, Feedback, ItemOutcome, Phase, Presentation, ReadingScore,
    Resolution, ResultOutcome, RubricOutcome, RubricResult, UnitSummary, UnscoredReason,
};

use crate::api::{
    ActivityBody, ActivityOutcomeView, ActivityPresentation, ActivityResultView, AudioLineView,
    CheckpointRowView, ContentPointView, ConversationSummaryView, DeterministicFeedbackView,
    DraftComparisonView, DraftErrorView, DraftFeedbackView, DraftResolution, DraftStatusView,
    EarlierErrorView, EngineFaultView, ErrorFindingView, ErrorPatternView, EvidenceStatusView,
    ItemFeedbackView, ItemOutcomeView, ObjectiveEvidenceView, PairChoice, PairPrompt,
    PronFindingsView, PronWordView, QuestionPrompt, QuestionResultView, ReadingScoreView,
    RubricDimensionView, RubricFeedbackView, SessionLife, TurnAnalysisView, TurnPhase,
    UnitSummaryView,
};

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

pub(crate) fn turn_phase(state: tutor_engine::TurnState) -> TurnPhase {
    use tutor_engine::TurnState;
    match state {
        TurnState::Listening => TurnPhase::Listening,
        TurnState::Transcribing => TurnPhase::Transcribing,
        TurnState::Speaking => TurnPhase::Speaking,
        TurnState::Waiting => TurnPhase::Waiting,
        TurnState::Replying => TurnPhase::Replying,
        TurnState::Thinking => TurnPhase::Thinking,
    }
}

pub(crate) fn fault_view(fault: tutor_engine::EngineFault) -> EngineFaultView {
    use tutor_engine::EngineFault;
    match fault {
        EngineFault::Microphone => EngineFaultView::Microphone,
        EngineFault::SpeechRecognition => EngineFaultView::SpeechRecognition,
        EngineFault::SpeechSynthesis => EngineFaultView::SpeechSynthesis,
        EngineFault::Playback => EngineFaultView::Playback,
    }
}

/// A phase as the life of the session, the turn state while it is active, and
/// the fault of an engine error.
pub(crate) fn phase_parts(
    phase: Phase,
) -> (SessionLife, Option<TurnPhase>, Option<EngineFaultView>) {
    match phase {
        Phase::Active { turn } => (SessionLife::Active, Some(turn_phase(turn)), None),
        Phase::Paused => (SessionLife::Paused, None, None),
        Phase::ProviderUnavailable => (SessionLife::ProviderUnavailable, None, None),
        Phase::EngineError { fault } => (SessionLife::EngineError, None, Some(fault_view(fault))),
        Phase::Ended { .. } => (SessionLife::Ended, None, None),
    }
}

fn audio_line(line: &AudioLine) -> AudioLineView {
    AudioLineView {
        speaker: line.speaker.clone(),
        text: line.text.clone(),
    }
}

fn audio_lines(lines: &[AudioLine]) -> Vec<AudioLineView> {
    lines.iter().map(audio_line).collect()
}

fn questions(items: &[tutor_engine::QuestionView]) -> Vec<QuestionPrompt> {
    items
        .iter()
        .map(|q| QuestionPrompt {
            stem: q.stem.clone(),
            options: q.options.clone(),
        })
        .collect()
}

fn body(body: &Body) -> ActivityBody {
    match body {
        Body::Mcq {
            passage,
            audio,
            stem,
            options,
        } => ActivityBody::Mcq {
            passage: passage.clone(),
            audio: audio.as_deref().map(audio_lines),
            stem: stem.clone(),
            options: options.clone(),
        },
        Body::GapFill { text, gaps } => ActivityBody::GapFill {
            text: text.clone(),
            gaps: count(*gaps),
        },
        Body::Reorder { tokens } => ActivityBody::Reorder {
            tokens: tokens.clone(),
        },
        Body::Match { left, right } => ActivityBody::Match {
            left: left.clone(),
            right: right.clone(),
        },
        Body::Dictation { audio } => ActivityBody::Dictation {
            audio: audio_lines(audio),
        },
        Body::MinimalPairsListen { items } => ActivityBody::MinimalPairsListen {
            items: items
                .iter()
                .map(|item| PairChoice {
                    options: item.options.to_vec(),
                    audio: audio_line(&item.audio),
                })
                .collect(),
        },
        Body::MinimalPairsSay { pairs } => ActivityBody::MinimalPairsSay {
            pairs: pairs
                .iter()
                .map(|p| PairPrompt {
                    a: p.a.clone(),
                    b: p.b.clone(),
                    focus: p.focus.clone(),
                })
                .collect(),
        },
        Body::ReadingSet {
            passage,
            questions: items,
        } => ActivityBody::ReadingSet {
            passage: passage.clone(),
            questions: questions(items),
        },
        Body::ListeningSet {
            audio,
            replays_allowed,
            questions: items,
        } => ActivityBody::ListeningSet {
            audio: audio_lines(audio),
            replays_allowed: u32::from(*replays_allowed),
            questions: questions(items),
        },
        Body::ErrorCorrection { sentence } => ActivityBody::ErrorCorrection {
            sentence: sentence.clone(),
        },
        Body::ReadAloud {
            text,
            focus_phonemes,
        } => ActivityBody::ReadAloud {
            text: text.clone(),
            focus_phonemes: focus_phonemes.clone(),
        },
        Body::Shadowing { title, lines } => ActivityBody::Shadowing {
            title: title.clone(),
            lines: audio_lines(lines),
        },
        Body::Production {
            prompt,
            content_points,
            min_words,
            max_words,
        } => ActivityBody::Production {
            prompt: prompt.clone(),
            content_points: content_points.clone(),
            min_words: *min_words,
            max_words: *max_words,
        },
        Body::Mediation { source_text, task } => ActivityBody::Mediation {
            source_text: source_text.clone(),
            task: task.clone(),
        },
        Body::Roleplay {
            scenario,
            tutor_role,
            learner_role,
            goals,
            max_turns,
        } => ActivityBody::Roleplay {
            scenario: scenario.clone(),
            tutor_role: tutor_role.clone(),
            learner_role: learner_role.clone(),
            goals: goals.clone(),
            max_turns: u32::from(*max_turns),
        },
    }
}

pub(crate) fn presentation(p: &Presentation) -> ActivityPresentation {
    ActivityPresentation {
        id: p.id.clone(),
        activity_type: p.activity_type,
        skill: p.skill,
        instructions: p.instructions.clone(),
        body: body(&p.body),
    }
}

fn item_outcome(outcome: ItemOutcome) -> ItemOutcomeView {
    match outcome {
        ItemOutcome::Correct => ItemOutcomeView::Correct,
        ItemOutcome::Spelling => ItemOutcomeView::Spelling,
        ItemOutcome::Wrong => ItemOutcomeView::Wrong,
        ItemOutcome::Blank => ItemOutcomeView::Blank,
    }
}

fn deterministic(feedback: &Feedback) -> DeterministicFeedbackView {
    DeterministicFeedbackView {
        score: feedback.score,
        correct: count(feedback.correct),
        total: count(feedback.total),
        passed: feedback.passed,
        items: feedback
            .items
            .iter()
            .map(|item| ItemFeedbackView {
                index: count(item.index),
                outcome: item_outcome(item.outcome),
                given: item.given.clone(),
                expected: item.expected.clone(),
                explanation: item.explanation.clone(),
            })
            .collect(),
        explanation: feedback.explanation.clone(),
    }
}

fn dimension_status(status: DimensionStatus) -> &'static str {
    match status {
        DimensionStatus::Scored => "scored",
        DimensionStatus::NeedsReview => "needs_review",
    }
}

fn content_points(points: &[tutor_engine::PointResult]) -> Vec<ContentPointView> {
    points
        .iter()
        .map(|p| ContentPointView {
            point: p.point.clone(),
            covered: p.covered,
            quote: p.quote.clone(),
        })
        .collect()
}

/// A rubric score of a unit activity: bands per dimension, joined over the runs.
pub(crate) fn rubric_outcome(outcome: &RubricOutcome) -> RubricFeedbackView {
    RubricFeedbackView {
        dimensions: outcome
            .dimensions
            .iter()
            .map(|d| RubricDimensionView {
                dimension: d.dimension.as_str().to_owned(),
                band: d.band,
                reason: d.reason.clone(),
                evidence_quotes: d.evidence_quotes.clone(),
                status: dimension_status(d.status).to_owned(),
                capped: d.capped,
            })
            .collect(),
        content_points: content_points(&outcome.content_points),
        on_task: outcome.on_task,
        feedback_en: outcome.feedback_en.clone(),
        feedback_l1: outcome.feedback_l1.clone(),
        confidence: outcome.confidence,
        runs: u32::from(outcome.runs),
    }
}

/// A rubric score of a writing draft: one run.
fn rubric_result(result: &RubricResult) -> RubricFeedbackView {
    RubricFeedbackView {
        dimensions: result
            .dimensions
            .iter()
            .map(|d| RubricDimensionView {
                dimension: d.dimension.as_str().to_owned(),
                band: d.band.map(f64::from),
                reason: d.reason.clone(),
                evidence_quotes: d.evidence_quotes.clone(),
                status: dimension_status(d.status).to_owned(),
                capped: d.capped,
            })
            .collect(),
        content_points: content_points(&result.content_points),
        on_task: result.on_task,
        feedback_en: result.feedback_en.clone(),
        feedback_l1: result.feedback_l1.clone(),
        confidence: result.confidence,
        runs: 1,
    }
}

pub(crate) fn pron_findings(report: &UtteranceReport) -> PronFindingsView {
    let words = report
        .words
        .iter()
        .map(|word| match word {
            WordResult::Scored {
                text,
                score,
                flagged_phonemes,
                ..
            } => PronWordView {
                word: text.clone(),
                score: *score,
                flagged_phonemes: count(*flagged_phonemes),
                checked: true,
            },
            WordResult::NotChecked { text, .. } | WordResult::NotScored { text } => PronWordView {
                word: text.clone(),
                score: None,
                flagged_phonemes: 0,
                checked: false,
            },
        })
        .collect();
    PronFindingsView {
        experimental: report.experimental,
        utterance_score: report.utterance_score,
        scores_calibrated: report.scores_calibrated,
        words,
        highlighted: report.highlighted_words.iter().map(|i| count(*i)).collect(),
    }
}

fn unscored_sentence(reason: &UnscoredReason) -> String {
    match reason {
        UnscoredReason::NoScorer => "This activity is practice and has no score.".to_owned(),
        UnscoredReason::RubricMissing { .. } => {
            "The rubric for this activity is not installed, so the response is stored and not scored."
                .to_owned()
        }
        UnscoredReason::EngineUnavailable { why } => {
            format!("Stored and not scored: {why}.")
        }
        UnscoredReason::InteractionRubricMissing => {
            "The interaction rubric of this level is not installed, so the roleplay is stored and not scored."
                .to_owned()
        }
    }
}

pub(crate) fn activity_result(result: &ActivityResult) -> ActivityResultView {
    let outcome = match &result.outcome {
        ResultOutcome::Deterministic(feedback) => ActivityOutcomeView::Deterministic {
            feedback: deterministic(feedback),
        },
        ResultOutcome::Rubric(outcome) => ActivityOutcomeView::Rubric {
            feedback: rubric_outcome(outcome),
        },
        ResultOutcome::Queued => ActivityOutcomeView::Queued,
        ResultOutcome::Pron(reports) => ActivityOutcomeView::Pron {
            findings: reports.iter().map(pron_findings).collect(),
        },
        ResultOutcome::Unscored(reason) => ActivityOutcomeView::Unscored {
            reason: unscored_sentence(reason),
        },
    };
    ActivityResultView {
        activity_id: result.activity_id.clone(),
        activity_type: result.activity_type,
        score: result.score,
        confidence: result.confidence,
        outcome,
    }
}

fn evidence_status(status: EvidenceStatus) -> EvidenceStatusView {
    match status {
        EvidenceStatus::Demonstrated => EvidenceStatusView::Demonstrated,
        EvidenceStatus::Partial => EvidenceStatusView::Partial,
        EvidenceStatus::NotDemonstrated => EvidenceStatusView::NotDemonstrated,
    }
}

pub(crate) fn analysis(record: &tutor_engine::AnalysedRecord) -> TurnAnalysisView {
    TurnAnalysisView {
        turn_seq: record.turn_seq,
        errors: record
            .analysis
            .errors
            .iter()
            .map(|e| ErrorFindingView {
                category: e.category.clone(),
                quote: e.quote.clone(),
                correction: e.correction.clone(),
                severity: e.severity.into(),
                addressed_in_reply: e.addressed_in_reply,
            })
            .collect(),
        objective_evidence: record
            .analysis
            .objective_evidence
            .iter()
            .map(|e| ObjectiveEvidenceView {
                objective_id: e.objective_id.clone(),
                status: evidence_status(e.status),
                quote: e.quote.clone(),
            })
            .collect(),
        note: record.analysis.note_for_next_turn.clone(),
        dropped: count(record.dropped),
    }
}

fn comparison(c: &DraftComparison) -> DraftComparisonView {
    let error = |e: &tutor_engine::DraftError| DraftErrorView {
        quote: e.quote.clone(),
        category: e.category.clone(),
    };
    DraftComparisonView {
        earlier: c
            .earlier
            .iter()
            .map(|e| EarlierErrorView {
                error: error(&e.error),
                resolution: match e.resolution {
                    Resolution::Fixed => DraftResolution::Fixed,
                    Resolution::Remaining => DraftResolution::Remaining,
                },
            })
            .collect(),
        new: c.new.iter().map(error).collect(),
    }
}

pub(crate) fn draft_feedback(feedback: &DraftFeedback) -> DraftFeedbackView {
    DraftFeedbackView {
        turn_seq: feedback.turn_seq,
        status: match feedback.status {
            DraftStatus::Analysed => DraftStatusView::Analysed,
            DraftStatus::RubricPending => DraftStatusView::RubricPending,
            DraftStatus::Pending => DraftStatusView::Pending,
        },
        analysis: feedback.analysis.as_ref().map(analysis),
        comparison: feedback.comparison.as_ref().map(comparison),
        rubric: feedback.rubric.as_ref().map(rubric_result),
    }
}

pub(crate) fn reading_score(score: &ReadingScore) -> ReadingScoreView {
    ReadingScoreView {
        correct: count(score.correct),
        total: count(score.total),
        score: score.score,
        questions: score
            .questions
            .iter()
            .map(|q| QuestionResultView {
                chosen: q.chosen.map(count),
                correct_index: count(q.correct_index),
                correct: q.correct,
                explanation_en: q.explanation_en.clone(),
                explanation_l1: q.explanation_l1.clone(),
            })
            .collect(),
    }
}

pub(crate) fn conversation_summary(summary: &ChatSummary) -> ConversationSummaryView {
    ConversationSummaryView {
        learner_turns: count(summary.learner_turns),
        top_errors: summary
            .top_errors
            .iter()
            .map(|e| ErrorPatternView {
                category: e.category.clone(),
                count: count(e.count),
                quote: e.quote.clone(),
                correction: e.correction.clone(),
            })
            .collect(),
        unanalysed_turns: summary.unanalysed_turns.clone(),
        analysis_unreliable: summary.analysis_unreliable,
    }
}

pub(crate) fn unit_summary(unit_id: &str, summary: &UnitSummary) -> UnitSummaryView {
    let report = &summary.checkpoint;
    UnitSummaryView {
        unit_id: unit_id.to_owned(),
        rows: report
            .rows
            .iter()
            .map(|r| CheckpointRowView {
                activity_id: r.activity_id.clone(),
                answered: r.answered,
                score: r.score,
            })
            .collect(),
        mean: report.outcome.mean,
        pass_mark: report.pass_mark,
        passed: report.outcome.passed,
        provisional: report.outcome.provisional,
        unit_status: summary.status.into(),
        answered: count(summary.answered),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tutor_engine::{EndReason, EngineFault, TurnState};

    #[test]
    fn every_phase_maps_to_a_life_and_only_an_active_one_has_a_turn_state() {
        let cases = [
            (
                Phase::Active {
                    turn: TurnState::Replying,
                },
                SessionLife::Active,
                Some(TurnPhase::Replying),
                None,
            ),
            (Phase::Paused, SessionLife::Paused, None, None),
            (
                Phase::ProviderUnavailable,
                SessionLife::ProviderUnavailable,
                None,
                None,
            ),
            (
                Phase::EngineError {
                    fault: EngineFault::Playback,
                },
                SessionLife::EngineError,
                None,
                Some(EngineFaultView::Playback),
            ),
            (
                Phase::Ended {
                    reason: EndReason::Finished,
                },
                SessionLife::Ended,
                None,
                None,
            ),
        ];
        for (phase, life, turn, fault) in cases {
            assert_eq!(phase_parts(phase), (life, turn, fault), "{phase:?}");
        }
    }

    #[test]
    fn a_presentation_keeps_the_audio_text_for_the_server_and_no_answer_exists_to_copy() {
        let unit = curriculum::load_unit_bytes(
            include_str!("../../../../curriculum/examples/a1-u01.example.json").as_bytes(),
        )
        .expect("the example unit loads")
        .unit;
        let mut seen_audio = false;
        for activity in &unit.activities {
            let shown = tutor_engine::present(&unit, activity).expect("presentable");
            let view = presentation(&shown);
            assert_eq!(view.id, shown.id);
            let json = serde_json::to_string(&view).expect("serialises");
            assert!(!json.contains("answer_index"), "{json}");
            if let ActivityBody::ListeningSet { audio, .. } = &view.body {
                seen_audio = !audio.is_empty();
            }
        }
        assert!(seen_audio, "the example unit has a listening set");
    }
}
