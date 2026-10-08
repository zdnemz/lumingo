//! The seam between the drills and the pronunciation engine.
//!
//! A drill (`read_aloud`, `minimal_pairs` in say mode, `shadowing`) knows its
//! reference text, so the activity runtime asks a [`DrillScorer`] to score one
//! recording against it. The trait is all the runtime sees: tests give it a
//! scripted scorer and need no model, and the real adapter,
//! [`PronEngineDrill`], wraps the posterior model, lexicon and calibration of
//! `pron-engine`. A missing model is an [`DrillError::Unavailable`], reported as
//! such and recorded as an unscored attempt, never as a zero.

use std::str::FromStr;
use std::sync::Mutex;

use pron_engine::{
    Arpabet, BoundPhoneMap, Calibration, Lexicon, Mode, ModelError, PosteriorModel, Reference,
    ScoreRequest, UtteranceReport, score_utterance,
};
use speech::{CancelFlag, EngineInfo};

/// One recording to score.
#[derive(Debug, Clone, PartialEq)]
pub struct DrillRequest {
    /// What the learner was asked to say, authored and known.
    pub reference_text: String,
    /// The ARPAbet symbols the drill targets, without stress digits.
    pub focus: Vec<String>,
    /// 16 kHz mono samples in -1..1.
    pub samples: Vec<f32>,
}

/// A scored recording.
#[derive(Debug, Clone, PartialEq)]
pub struct DrillReport {
    pub report: UtteranceReport,
    /// Milliseconds per posterior frame, to turn aligned frames into times.
    pub frame_ms: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum DrillError {
    /// The engine is not installed or not built in. The message says why and
    /// holds no learner text.
    #[error("the pronunciation engine is not available: {0}")]
    Unavailable(String),
    #[error("the pronunciation analysis was cancelled")]
    Cancelled,
    /// The engine ran and failed, or the request could not be scored.
    #[error("the pronunciation analysis failed: {0}")]
    Failed(String),
}

/// Scores a recording of a known text. Blocking and CPU heavy: call it from a
/// blocking thread, never from an async worker.
pub trait DrillScorer: Send + Sync {
    /// Which engine and model scored, for the evidence rows.
    fn engine(&self) -> EngineInfo;

    /// The version of the threshold set the scores were produced with.
    fn threshold_set_version(&self) -> String;

    fn score(&self, request: &DrillRequest, cancel: &CancelFlag)
    -> Result<DrillReport, DrillError>;
}

/// The real scorer: the phoneme model, the lexicon, the phone map and the
/// calibration of `pron-engine`, behind the trait.
///
/// UNVERIFIED with a real model: the build container has none. The path through
/// it is tested with a scripted [`PosteriorModel`].
pub struct PronEngineDrill<M> {
    model: Mutex<M>,
    lexicon: Lexicon,
    map: BoundPhoneMap,
    calibration: Calibration,
    info: EngineInfo,
    threshold_set_version: String,
    frame_ms: u32,
}

impl<M: PosteriorModel + Send> PronEngineDrill<M> {
    pub fn new(
        model: M,
        lexicon: Lexicon,
        map: BoundPhoneMap,
        calibration: Calibration,
        info: EngineInfo,
        threshold_set_version: impl Into<String>,
        frame_ms: u32,
    ) -> Self {
        Self {
            model: Mutex::new(model),
            lexicon,
            map,
            calibration,
            info,
            threshold_set_version: threshold_set_version.into(),
            frame_ms,
        }
    }
}

impl<M: PosteriorModel + Send> DrillScorer for PronEngineDrill<M> {
    fn engine(&self) -> EngineInfo {
        self.info.clone()
    }

    fn threshold_set_version(&self) -> String {
        self.threshold_set_version.clone()
    }

    fn score(
        &self,
        request: &DrillRequest,
        cancel: &CancelFlag,
    ) -> Result<DrillReport, DrillError> {
        let focus: Vec<Arpabet> = request
            .focus
            .iter()
            .map(|symbol| {
                Arpabet::from_str(symbol)
                    .map_err(|_| DrillError::Failed(format!("{symbol} is not an ARPAbet symbol")))
            })
            .collect::<Result<_, _>>()?;
        let reference = Reference::from_text(&request.reference_text, &self.lexicon);
        let mut model = self
            .model
            .lock()
            .map_err(|_| DrillError::Failed("the phoneme model is poisoned".to_owned()))?;
        let posteriors =
            model
                .log_posteriors(&request.samples, cancel)
                .map_err(|error| match error {
                    ModelError::Unavailable { reason } => DrillError::Unavailable(reason),
                    ModelError::Cancelled => DrillError::Cancelled,
                    other => DrillError::Failed(other.to_string()),
                })?;
        let report = score_utterance(&ScoreRequest {
            reference: &reference,
            posteriors: &posteriors,
            map: &self.map,
            vocab: model.vocab(),
            calibration: &self.calibration,
            mode: Mode::Drill,
            focus: &focus,
        })
        .map_err(|error| DrillError::Failed(error.to_string()))?;
        Ok(DrillReport {
            report,
            frame_ms: self.frame_ms,
        })
    }
}
