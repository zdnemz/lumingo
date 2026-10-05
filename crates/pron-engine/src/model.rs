//! The seam between audio and the posterior matrix.
//!
//! Everything downstream of [`PosteriorModel`] works on a plain
//! [`LogPosteriors`] matrix. The one real implementation (ONNX Runtime through
//! `ort`) lives behind the off-by-default `ort-backend` feature; the default
//! build carries only [`UnavailableModel`], which says so instead of pretending.

use speech::CancelFlag;

use crate::posteriors::{LogPosteriors, PosteriorsError};
use crate::vocab::ModelVocab;

#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("the phoneme model is not available: {reason}")]
    Unavailable { reason: String },
    #[error("the phoneme analysis was cancelled")]
    Cancelled,
    #[error("the audio cannot be analysed: {0}")]
    BadInput(String),
    #[error("the phoneme model failed: {0}")]
    Inference(String),
    #[error("the phoneme model returned an unexpected output shape {shape:?}: {reason}")]
    OutputShape { shape: Vec<i64>, reason: String },
    #[error(transparent)]
    Posteriors(#[from] PosteriorsError),
}

/// Turns 16 kHz mono audio into log posteriors.
///
/// Inference is blocking and CPU heavy: call it from a dedicated thread, never
/// from a Tokio worker thread.
pub trait PosteriorModel {
    fn vocab(&self) -> &ModelVocab;

    /// `samples` are mono `f32` at [`speech::SPEECH_SAMPLE_RATE`], in -1..1.
    fn log_posteriors(
        &mut self,
        samples: &[f32],
        cancel: &CancelFlag,
    ) -> Result<LogPosteriors, ModelError>;
}

/// Stands in for the phoneme model when it is not installed or not compiled in.
/// It reports that plainly and produces nothing; it never invents a matrix.
#[derive(Debug)]
pub struct UnavailableModel {
    vocab: ModelVocab,
    reason: String,
}

impl UnavailableModel {
    pub fn new(vocab: ModelVocab, reason: impl Into<String>) -> Self {
        Self {
            vocab,
            reason: reason.into(),
        }
    }
}

impl PosteriorModel for UnavailableModel {
    fn vocab(&self) -> &ModelVocab {
        &self.vocab
    }

    fn log_posteriors(
        &mut self,
        _samples: &[f32],
        _cancel: &CancelFlag,
    ) -> Result<LogPosteriors, ModelError> {
        Err(ModelError::Unavailable {
            reason: self.reason.clone(),
        })
    }
}

/// Zero mean and unit variance over the whole utterance, the preprocessing
/// wav2vec2-style checkpoints are trained with (`do_normalize` in their
/// feature-extractor config). The small epsilon keeps constant audio finite.
///
/// Whether the chosen model wants this is a property of that model and is
/// checked when it is chosen (S1-07); the function is here because it is pure
/// and testable.
pub fn normalise_waveform(samples: &[f32]) -> Vec<f32> {
    if samples.is_empty() {
        return Vec::new();
    }
    let n = samples.len() as f64;
    let mean = samples.iter().map(|s| f64::from(*s)).sum::<f64>() / n;
    let var = samples
        .iter()
        .map(|s| {
            let d = f64::from(*s) - mean;
            d * d
        })
        .sum::<f64>()
        / n;
    let scale = 1.0 / (var + 1e-7).sqrt();
    samples
        .iter()
        .map(|s| ((f64::from(*s) - mean) * scale) as f32)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalised_audio_has_zero_mean_and_unit_variance() {
        let samples: Vec<f32> = (0..1000)
            .map(|i| ((i as f32) * 0.05).sin() * 0.3 + 0.1)
            .collect();
        let out = normalise_waveform(&samples);
        let n = out.len() as f64;
        let mean = out.iter().map(|v| f64::from(*v)).sum::<f64>() / n;
        let var = out
            .iter()
            .map(|v| (f64::from(*v) - mean).powi(2))
            .sum::<f64>()
            / n;
        assert!(mean.abs() < 1e-4, "mean {mean}");
        assert!((var - 1.0).abs() < 1e-3, "variance {var}");
    }

    #[test]
    fn constant_and_empty_audio_stay_finite() {
        let out = normalise_waveform(&[0.25; 64]);
        assert!(out.iter().all(|v| v.is_finite() && v.abs() < 1e-3));
        assert!(normalise_waveform(&[]).is_empty());
    }

    #[test]
    fn the_unavailable_model_refuses_and_says_why() {
        let vocab = ModelVocab::from_labels(vec!["<pad>".into(), "a".into()], "<pad>").expect("v");
        let mut model = UnavailableModel::new(vocab, "built without the ort-backend feature");
        assert_eq!(model.vocab().len(), 2);
        let err = model
            .log_posteriors(&[0.0; 1000], &CancelFlag::new())
            .expect_err("must refuse");
        assert!(err.to_string().contains("ort-backend"));
    }
}
