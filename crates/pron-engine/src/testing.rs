//! Builders for synthetic posterior matrices, used only by unit tests.
//!
//! A synthetic matrix is written down frame by frame, so the correct alignment
//! and the correct GOP of every phone are known by construction.

use crate::arpabet::Arpabet;
use crate::phone_map::{BoundPhoneMap, PhoneMap};
use crate::posteriors::LogPosteriors;
use crate::vocab::ModelVocab;

/// Probability given to the intended label of a frame. The rest is spread
/// evenly over the other labels.
pub(crate) const DOMINANT: f32 = 0.9;

/// The bundled map bound to a vocabulary that holds exactly the first label of
/// every symbol plus `<pad>` as the blank. Column 0 is the blank.
pub(crate) fn bound_map() -> (BoundPhoneMap, ModelVocab) {
    let map = PhoneMap::bundled_candidate().expect("bundled map loads");
    let mut labels = vec!["<pad>".to_owned()];
    for symbol in Arpabet::ALL {
        let first = map.labels_of(symbol)[0].clone();
        if !labels.contains(&first) {
            labels.push(first);
        }
    }
    let vocab = ModelVocab::from_labels(labels, "<pad>").expect("vocab");
    let bound = map.bind(&vocab).expect("binds");
    (bound, vocab)
}

pub(crate) struct Synthetic {
    columns: usize,
    blank: usize,
    column_of: Vec<(Arpabet, usize)>,
    frames: Vec<Vec<f32>>,
}

impl Synthetic {
    pub(crate) fn new(vocab: &ModelVocab) -> Self {
        let map = PhoneMap::bundled_candidate().expect("bundled map loads");
        let column_of = Arpabet::ALL
            .iter()
            .map(|s| {
                let first = &map.labels_of(*s)[0];
                (
                    *s,
                    vocab.index_of(first).expect("first label is in the vocab"),
                )
            })
            .collect();
        Self {
            columns: vocab.len(),
            blank: vocab.blank(),
            column_of,
            frames: Vec::new(),
        }
    }

    pub(crate) fn column(&self, symbol: Arpabet) -> usize {
        self.column_of
            .iter()
            .find(|(s, _)| *s == symbol)
            .map(|(_, c)| *c)
            .expect("every symbol has a column")
    }

    /// `n` frames where `column` has probability `p` and the rest is uniform.
    pub(crate) fn column_frames(&mut self, column: usize, n: usize, p: f32) -> &mut Self {
        self.mixed(&[(column, p)], n)
    }

    /// `n` identical frames with the given probabilities on some columns; the
    /// remaining probability is spread evenly over the other columns.
    pub(crate) fn mixed(&mut self, given: &[(usize, f32)], n: usize) -> &mut Self {
        let used: f32 = given.iter().map(|(_, p)| *p).sum();
        assert!(
            used < 1.0 && used > 0.0,
            "given probabilities must sum to (0, 1)"
        );
        let rest = (1.0 - used) / (self.columns - given.len()) as f32;
        let mut row = vec![rest; self.columns];
        for (c, p) in given {
            row[*c] = *p;
        }
        for _ in 0..n {
            self.frames.push(row.clone());
        }
        self
    }

    pub(crate) fn blank(&mut self, n: usize) -> &mut Self {
        let b = self.blank;
        self.column_frames(b, n, DOMINANT)
    }

    pub(crate) fn phone(&mut self, symbol: Arpabet, n: usize) -> &mut Self {
        let c = self.column(symbol);
        self.column_frames(c, n, DOMINANT)
    }

    pub(crate) fn matrix(&self) -> LogPosteriors {
        let data: Vec<f32> = self
            .frames
            .iter()
            .flat_map(|row| row.iter().map(|p| p.ln()))
            .collect();
        LogPosteriors::new(data, self.frames.len(), self.columns).expect("valid matrix")
    }
}
