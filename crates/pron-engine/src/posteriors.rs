//! The acoustic model's output as a plain matrix.
//!
//! Alignment, GOP and scoring only see this type, so all of them are tested on
//! synthetic matrices and never need a model.

use serde::{Deserialize, Serialize};

/// Why a matrix could not be built.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PosteriorsError {
    #[error("a matrix of {frames} frames by {labels} labels needs {expected} values, got {got}")]
    Shape {
        frames: usize,
        labels: usize,
        expected: usize,
        got: usize,
    },
    #[error("a matrix needs at least one label column")]
    NoLabels,
    #[error("value at frame {frame}, label {label} is NaN or +infinity")]
    NotFinite { frame: usize, label: usize },
    #[error("the posteriors file is not valid JSON of the expected shape: {0}")]
    Json(String),
}

/// The JSON form used by the `pron` command line to score a matrix that was
/// produced elsewhere. `null` stands for probability zero (`-inf`), which JSON
/// cannot spell.
#[derive(Debug, Serialize, Deserialize)]
struct PosteriorsFile {
    frames: usize,
    labels: usize,
    log_posteriors: Vec<Option<f32>>,
}

/// Log posteriors, frames by labels, row-major. `-inf` is allowed (probability
/// zero); NaN and `+inf` are not.
#[derive(Debug, Clone, PartialEq)]
pub struct LogPosteriors {
    data: Vec<f32>,
    frames: usize,
    labels: usize,
}

impl LogPosteriors {
    /// Wraps values that are already log probabilities.
    pub fn new(data: Vec<f32>, frames: usize, labels: usize) -> Result<Self, PosteriorsError> {
        if labels == 0 {
            return Err(PosteriorsError::NoLabels);
        }
        let expected = frames.saturating_mul(labels);
        if data.len() != expected {
            return Err(PosteriorsError::Shape {
                frames,
                labels,
                expected,
                got: data.len(),
            });
        }
        if let Some(i) = data.iter().position(|v| v.is_nan() || *v == f32::INFINITY) {
            return Err(PosteriorsError::NotFinite {
                frame: i / labels,
                label: i % labels,
            });
        }
        Ok(Self {
            data,
            frames,
            labels,
        })
    }

    /// Applies a numerically stable log-softmax to each frame of raw logits.
    pub fn from_logits(
        mut logits: Vec<f32>,
        frames: usize,
        labels: usize,
    ) -> Result<Self, PosteriorsError> {
        if labels > 0 && logits.len() == frames.saturating_mul(labels) {
            for row in logits.chunks_exact_mut(labels) {
                log_softmax_in_place(row);
            }
        }
        Self::new(logits, frames, labels)
    }

    /// Reads the JSON form written by [`LogPosteriors::to_json`].
    pub fn from_json(text: &str) -> Result<Self, PosteriorsError> {
        let file: PosteriorsFile =
            serde_json::from_str(text).map_err(|e| PosteriorsError::Json(e.to_string()))?;
        let data = file
            .log_posteriors
            .into_iter()
            .map(|v| v.unwrap_or(f32::NEG_INFINITY))
            .collect();
        Self::new(data, file.frames, file.labels)
    }

    pub fn to_json(&self) -> Result<String, PosteriorsError> {
        let file = PosteriorsFile {
            frames: self.frames,
            labels: self.labels,
            log_posteriors: self
                .data
                .iter()
                .map(|v| (*v != f32::NEG_INFINITY).then_some(*v))
                .collect(),
        };
        serde_json::to_string(&file).map_err(|e| PosteriorsError::Json(e.to_string()))
    }

    pub fn frames(&self) -> usize {
        self.frames
    }

    pub fn labels(&self) -> usize {
        self.labels
    }

    /// One frame, or `None` when `frame` is out of range.
    pub fn row(&self, frame: usize) -> Option<&[f32]> {
        let start = frame.checked_mul(self.labels)?;
        let end = start.checked_add(self.labels)?;
        self.data.get(start..end)
    }

    /// The value at one frame and label, or `None` out of range.
    pub fn get(&self, frame: usize, label: usize) -> Option<f32> {
        self.row(frame)?.get(label).copied()
    }
}

fn log_softmax_in_place(row: &mut [f32]) {
    let max = row.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    if !max.is_finite() {
        return;
    }
    let sum: f32 = row.iter().map(|v| (v - max).exp()).sum();
    let log_sum = max + sum.ln();
    for v in row {
        *v -= log_sum;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_and_values_are_checked() {
        assert!(LogPosteriors::new(vec![-1.0; 6], 2, 3).is_ok());
        assert_eq!(
            LogPosteriors::new(vec![-1.0; 5], 2, 3),
            Err(PosteriorsError::Shape {
                frames: 2,
                labels: 3,
                expected: 6,
                got: 5
            })
        );
        assert_eq!(
            LogPosteriors::new(vec![], 0, 0),
            Err(PosteriorsError::NoLabels)
        );
        assert_eq!(
            LogPosteriors::new(vec![-1.0, f32::NAN, -1.0, -1.0], 2, 2),
            Err(PosteriorsError::NotFinite { frame: 0, label: 1 })
        );
        assert!(LogPosteriors::new(vec![f32::NEG_INFINITY, 0.0], 1, 2).is_ok());
        assert_eq!(
            LogPosteriors::new(vec![0.0, 0.0, 0.0, f32::INFINITY], 2, 2),
            Err(PosteriorsError::NotFinite { frame: 1, label: 1 })
        );
    }

    #[test]
    fn json_round_trips_including_zero_probability() {
        let m =
            LogPosteriors::new(vec![-0.5, f32::NEG_INFINITY, -2.0, -3.5], 2, 2).expect("matrix");
        let text = m.to_json().expect("json");
        assert!(text.contains("null"));
        assert_eq!(LogPosteriors::from_json(&text).expect("parses"), m);
        assert!(matches!(
            LogPosteriors::from_json("{\"frames\":1}"),
            Err(PosteriorsError::Json(_))
        ));
        assert!(matches!(
            LogPosteriors::from_json("{\"frames\":2,\"labels\":2,\"log_posteriors\":[0.0]}"),
            Err(PosteriorsError::Shape { .. })
        ));
    }

    #[test]
    fn zero_frames_is_a_valid_empty_matrix() {
        let m = LogPosteriors::new(vec![], 0, 4).expect("empty matrix");
        assert_eq!(m.frames(), 0);
        assert!(m.row(0).is_none());
    }

    #[test]
    fn rows_and_cells_are_addressable_and_bounded() {
        let m = LogPosteriors::new(vec![-1.0, -2.0, -3.0, -4.0], 2, 2).expect("matrix");
        assert_eq!(m.row(1), Some(&[-3.0, -4.0][..]));
        assert_eq!(m.get(0, 1), Some(-2.0));
        assert_eq!(m.get(2, 0), None);
        assert_eq!(m.get(0, 2), None);
        assert!(m.row(usize::MAX).is_none());
    }

    #[test]
    fn log_softmax_rows_sum_to_one_in_probability() {
        let m = LogPosteriors::from_logits(vec![1.0, 2.0, 3.0, 100.0, 0.0, -100.0], 2, 3)
            .expect("matrix");
        for t in 0..2 {
            let total: f32 = m.row(t).expect("row").iter().map(|v| v.exp()).sum();
            assert!((total - 1.0).abs() < 1e-5, "frame {t} sums to {total}");
        }
        let row0 = m.row(0).expect("row");
        assert!(row0[2] > row0[1] && row0[1] > row0[0]);
    }
}
