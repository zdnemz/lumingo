//! ONNX Runtime adapter for the phoneme model (feature `ort-backend`).
//!
//! UNVERIFIED. This compiles against `ort` 2.0.0-rc.13 (checked with
//! `cargo check --features ort-backend`) but has never run: the build container
//! has no ONNX Runtime library, no phoneme model, and cannot download either.
//! Assumptions that only a real model can confirm:
//!
//! - one input, a `[1, samples]` float32 waveform, normalised as
//!   [`normalise_waveform`] does;
//! - the first output is `[1, frames, labels]` float32 logits or log
//!   probabilities (log-softmax is idempotent, so either works);
//! - `labels` equals the size of the vocabulary file.
//!
//! The owner runs the first real inference on Windows; see the crate README.

use std::path::{Path, PathBuf};

use ort::session::Session;
use ort::value::Tensor;
use speech::CancelFlag;

use crate::model::{ModelError, PosteriorModel, normalise_waveform};
use crate::posteriors::LogPosteriors;
use crate::vocab::ModelVocab;

/// Shortest input the adapter will send. wav2vec2-style convolution stacks have
/// a 400-sample receptive field; shorter audio produces no frame at all.
const MIN_SAMPLES: usize = 400;

fn inference(e: impl std::fmt::Display) -> ModelError {
    ModelError::Inference(e.to_string())
}

/// Where to load the ONNX Runtime library from: the explicit path, else
/// `ORT_DYLIB_PATH`, else the platform's default file name (resolved by the
/// dynamic loader). Loading it here, not lazily, turns a missing library into
/// an error instead of the panic `ort` raises when it is first used.
fn runtime_library(explicit: Option<&Path>) -> PathBuf {
    if let Some(path) = explicit {
        return path.to_owned();
    }
    match std::env::var("ORT_DYLIB_PATH") {
        Ok(p) if !p.is_empty() => PathBuf::from(p),
        _ if cfg!(windows) => PathBuf::from("onnxruntime.dll"),
        _ if cfg!(target_os = "macos") => PathBuf::from("libonnxruntime.dylib"),
        _ => PathBuf::from("libonnxruntime.so"),
    }
}

pub struct OrtPosteriorModel {
    session: Session,
    vocab: ModelVocab,
}

impl OrtPosteriorModel {
    /// Loads the runtime library and the model. `intra_threads` is the number
    /// of CPU threads inference may use; the caller lowers it in deferred mode.
    pub fn load(
        runtime: Option<&Path>,
        model: &Path,
        vocab: ModelVocab,
        intra_threads: usize,
    ) -> Result<Self, ModelError> {
        let library = runtime_library(runtime);
        let environment = ort::init_from(&library).map_err(|e| ModelError::Unavailable {
            reason: format!("cannot load the ONNX Runtime library: {e}"),
        })?;
        // `false` means an environment already exists in this process, which is
        // fine: the first one wins.
        let _committed = environment.commit();

        let mut builder = Session::builder()
            .map_err(inference)?
            .with_intra_threads(intra_threads.max(1))
            .map_err(inference)?;
        let session = builder
            .commit_from_file(model)
            .map_err(|e| ModelError::Unavailable {
                reason: format!("cannot load the model {}: {e}", model.display()),
            })?;
        if session.inputs().len() != 1 {
            return Err(ModelError::Unavailable {
                reason: format!(
                    "the model has {} inputs; the adapter supports exactly one waveform input",
                    session.inputs().len()
                ),
            });
        }
        if session.outputs().is_empty() {
            return Err(ModelError::Unavailable {
                reason: "the model has no outputs".to_owned(),
            });
        }
        Ok(Self { session, vocab })
    }
}

impl PosteriorModel for OrtPosteriorModel {
    fn vocab(&self) -> &ModelVocab {
        &self.vocab
    }

    /// A running inference is not interrupted: the flag is checked before it
    /// starts and after it returns.
    fn log_posteriors(
        &mut self,
        samples: &[f32],
        cancel: &CancelFlag,
    ) -> Result<LogPosteriors, ModelError> {
        if cancel.is_cancelled() {
            return Err(ModelError::Cancelled);
        }
        if samples.len() < MIN_SAMPLES {
            return Err(ModelError::BadInput(format!(
                "{} samples is shorter than the model's {MIN_SAMPLES}-sample window",
                samples.len()
            )));
        }
        let input = normalise_waveform(samples);
        let tensor = Tensor::from_array(([1usize, input.len()], input.into_boxed_slice()))
            .map_err(inference)?;
        let outputs = self.session.run(ort::inputs![tensor]).map_err(inference)?;
        if cancel.is_cancelled() {
            return Err(ModelError::Cancelled);
        }
        let first = outputs
            .values()
            .next()
            .ok_or_else(|| ModelError::Inference("the model returned no output".to_owned()))?;
        let (shape, data) = first.try_extract_tensor::<f32>().map_err(inference)?;
        let dims: Vec<i64> = shape.iter().copied().collect();
        let [batch, frames, labels] = dims[..] else {
            return Err(ModelError::OutputShape {
                shape: dims,
                reason: "expected [1, frames, labels]".to_owned(),
            });
        };
        if batch != 1 || frames < 0 || usize::try_from(labels).ok() != Some(self.vocab.len()) {
            return Err(ModelError::OutputShape {
                shape: dims,
                reason: format!(
                    "expected [1, frames, {}] to match the vocabulary",
                    self.vocab.len()
                ),
            });
        }
        let frames = usize::try_from(frames).map_err(inference)?;
        Ok(LogPosteriors::from_logits(
            data.to_vec(),
            frames,
            self.vocab.len(),
        )?)
    }
}
