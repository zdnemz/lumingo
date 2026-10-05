//! Goodness of pronunciation for one aligned phone (ASSESSMENT_SPEC 7.1
//! step 5).
//!
//! `GOP(p) = mean over t in T(p) of [ log P_t(p) - max over q of log P_t(q) ]`,
//! with the blank excluded from `q`. When a phone has several acceptable model
//! labels, `log P_t(p)` is the best of them, which keeps GOP at most zero.

use std::ops::Range;

use crate::posteriors::LogPosteriors;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum GopError {
    #[error("the frame span {start}..{end} is empty or outside the {frames} frames")]
    BadSpan {
        start: usize,
        end: usize,
        frames: usize,
    },
    #[error("a phone needs at least one label column")]
    NoColumns,
    #[error("column {0} is outside the matrix or is the blank")]
    BadColumn(usize),
    /// Every non-blank label has probability zero at this frame, so there is no
    /// competitor to compare with. A forced alignment never places a phone here.
    #[error("frame {0} gives no probability to any label but the blank")]
    NoEvidence(usize),
}

/// The label that won most frames of a phone when it was not the expected one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeardColumn {
    pub column: usize,
    /// Frames of the phone's span on which this column was the best non-blank.
    pub frames_won: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhoneGop {
    /// At most 0. Near 0 is good.
    pub gop: f32,
    /// Number of frames the mean was taken over.
    pub frames: usize,
    /// Set only when the winning column is not one of the phone's own columns.
    pub heard: Option<HeardColumn>,
}

/// Computes GOP for a phone aligned to `span`.
///
/// `columns` are the model columns that count as the expected phone. `blank` is
/// excluded from the competing labels. The "heard" column is the one that is
/// the best non-blank on most frames of the span (the lowest column index wins a
/// tie, so the result does not depend on hash order or float noise); it is
/// reported only if it is not one of `columns`.
pub fn gop(
    posteriors: &LogPosteriors,
    columns: &[usize],
    blank: usize,
    span: Range<usize>,
) -> Result<PhoneGop, GopError> {
    if columns.is_empty() {
        return Err(GopError::NoColumns);
    }
    if let Some(bad) = columns
        .iter()
        .find(|c| **c >= posteriors.labels() || **c == blank)
    {
        return Err(GopError::BadColumn(*bad));
    }
    if span.start >= span.end || span.end > posteriors.frames() {
        return Err(GopError::BadSpan {
            start: span.start,
            end: span.end,
            frames: posteriors.frames(),
        });
    }

    let mut total = 0.0f64;
    let mut wins = vec![0usize; posteriors.labels()];
    for t in span.clone() {
        let row = posteriors.row(t).ok_or(GopError::BadSpan {
            start: span.start,
            end: span.end,
            frames: posteriors.frames(),
        })?;
        let mut best_col = None;
        let mut best = f32::NEG_INFINITY;
        for (l, v) in row.iter().enumerate() {
            if l != blank && *v > best {
                best = *v;
                best_col = Some(l);
            }
        }
        let Some(best_col) = best_col else {
            return Err(GopError::NoEvidence(t));
        };
        let expected = columns
            .iter()
            .map(|c| row[*c])
            .fold(f32::NEG_INFINITY, f32::max);
        total += f64::from(expected - best);
        wins[best_col] += 1;
    }

    let frames = span.len();
    let (winner, frames_won) =
        wins.iter().enumerate().fold(
            (0usize, 0usize),
            |acc, (c, n)| if *n > acc.1 { (c, *n) } else { acc },
        );
    let heard = (!columns.contains(&winner)).then_some(HeardColumn {
        column: winner,
        frames_won,
    });
    let gop = (total / frames as f64) as f32;
    Ok(PhoneGop { gop, frames, heard })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arpabet::Arpabet::*;
    use crate::testing::{Synthetic, bound_map};

    fn approx(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-4, "{a} != {b}");
    }

    #[test]
    fn a_confident_correct_phone_has_gop_zero_and_no_heard_label() {
        let (map, vocab) = bound_map();
        let mut s = Synthetic::new(&vocab);
        s.blank(1).phone(K, 3).blank(1);
        let m = s.matrix();
        let r = gop(&m, map.columns_of(K), map.blank(), 1..4).expect("gop");
        approx(r.gop, 0.0);
        assert_eq!(r.frames, 3);
        assert_eq!(r.heard, None);
    }

    #[test]
    fn gop_is_log_ratio_to_the_best_competitor_known_by_construction() {
        let (map, vocab) = bound_map();
        let mut s = Synthetic::new(&vocab);
        let k = s.column(K);
        let g = s.column(G);
        // Expected K at 0.3, competitor G at 0.6: GOP = ln(0.3/0.6) = -ln 2.
        s.mixed(&[(k, 0.3), (g, 0.6)], 4);
        let m = s.matrix();
        let r = gop(&m, map.columns_of(K), map.blank(), 0..4).expect("gop");
        approx(r.gop, -(2.0f32).ln());
        let heard = r.heard.expect("G wins every frame");
        assert_eq!(heard.column, g);
        assert_eq!(heard.frames_won, 4);
    }

    #[test]
    fn gop_is_the_mean_over_frames() {
        let (map, vocab) = bound_map();
        let mut s = Synthetic::new(&vocab);
        let k = s.column(K);
        let g = s.column(G);
        s.mixed(&[(k, 0.6), (g, 0.3)], 1); // expected wins: ratio 1 -> 0
        s.mixed(&[(k, 0.2), (g, 0.6)], 3); // ratio 1/3 -> -ln 3 each
        let m = s.matrix();
        let r = gop(&m, map.columns_of(K), map.blank(), 0..4).expect("gop");
        approx(r.gop, -3.0 * (3.0f32).ln() / 4.0);
        // G wins 3 of 4 frames.
        let heard = r.heard.expect("heard");
        assert_eq!((heard.column, heard.frames_won), (g, 3));
    }

    #[test]
    fn heard_is_none_when_the_expected_phone_wins_most_frames() {
        let (map, vocab) = bound_map();
        let mut s = Synthetic::new(&vocab);
        let k = s.column(K);
        let g = s.column(G);
        s.mixed(&[(k, 0.6), (g, 0.3)], 3);
        s.mixed(&[(k, 0.2), (g, 0.6)], 1);
        let m = s.matrix();
        let r = gop(&m, map.columns_of(K), map.blank(), 0..4).expect("gop");
        assert_eq!(r.heard, None);
        assert!(r.gop < 0.0);
    }

    #[test]
    fn the_blank_is_not_a_competitor() {
        let (map, vocab) = bound_map();
        let mut s = Synthetic::new(&vocab);
        let k = s.column(K);
        // The blank holds 0.9, K 0.05: against blank GOP would be -2.9, but the
        // blank is excluded, so K (the best non-blank) wins and GOP is 0.
        s.mixed(&[(0, 0.9), (k, 0.05)], 2);
        let m = s.matrix();
        let r = gop(&m, map.columns_of(K), map.blank(), 0..2).expect("gop");
        approx(r.gop, 0.0);
        assert_eq!(r.heard, None);
    }

    #[test]
    fn gop_is_never_positive() {
        let (map, vocab) = bound_map();
        for symbol in [K, AE, SH, M, L, W, JH] {
            let mut s = Synthetic::new(&vocab);
            let other = s.column(if symbol == S { Z } else { S });
            let me = s.column(symbol);
            s.mixed(&[(me, 0.35), (other, 0.5)], 2);
            s.mixed(&[(me, 0.7)], 2);
            let m = s.matrix();
            let r = gop(&m, map.columns_of(symbol), map.blank(), 0..4).expect("gop");
            assert!(r.gop <= 0.0, "{symbol}: {}", r.gop);
        }
    }

    #[test]
    fn the_best_of_several_acceptable_labels_is_used() {
        // T accepts "t" and the flap. Posterior mass on the flap alone must
        // count as T and must not be reported as a substituted sound.
        let map = crate::phone_map::PhoneMap::bundled_candidate().expect("map");
        let mut labels = vec!["<pad>".to_owned()];
        for symbol in crate::arpabet::Arpabet::ALL {
            for l in map.labels_of(symbol) {
                if !labels.contains(l) {
                    labels.push(l.clone());
                }
            }
        }
        let vocab = crate::vocab::ModelVocab::from_labels(labels, "<pad>").expect("vocab");
        let bound = map.bind(&vocab).expect("binds");
        let mut s = Synthetic::new(&vocab);
        let flap = vocab.index_of("ɾ").expect("flap");
        s.mixed(&[(flap, 0.8)], 3);
        let m = s.matrix();
        let r = gop(&m, bound.columns_of(T), bound.blank(), 0..3).expect("gop");
        approx(r.gop, 0.0);
        assert_eq!(r.heard, None);
    }

    #[test]
    fn ties_for_heard_go_to_the_lowest_column() {
        let (map, vocab) = bound_map();
        let mut s = Synthetic::new(&vocab);
        let k = s.column(K);
        let a = s.column(AE);
        let b = s.column(B);
        s.mixed(&[(k, 0.1), (a, 0.4)], 1);
        s.mixed(&[(k, 0.1), (b, 0.4)], 1);
        let m = s.matrix();
        let r = gop(&m, map.columns_of(K), map.blank(), 0..2).expect("gop");
        assert_eq!(r.heard.map(|h| h.column), Some(a.min(b)));
    }

    #[test]
    fn bad_inputs_are_typed_errors() {
        let (map, vocab) = bound_map();
        let mut s = Synthetic::new(&vocab);
        s.phone(K, 3);
        let m = s.matrix();
        let cols = map.columns_of(K);
        assert!(matches!(gop(&m, &[], 0, 0..1), Err(GopError::NoColumns)));
        assert!(matches!(
            gop(&m, cols, 0, 2..2),
            Err(GopError::BadSpan { .. })
        ));
        assert!(matches!(
            gop(&m, cols, 0, 0..4),
            Err(GopError::BadSpan { .. })
        ));
        assert_eq!(gop(&m, &[0], 0, 0..1), Err(GopError::BadColumn(0)));
        assert_eq!(gop(&m, &[999], 0, 0..1), Err(GopError::BadColumn(999)));
        // A frame where only the blank has probability.
        let mut only_blank = vec![f32::NEG_INFINITY; vocab.len()];
        only_blank[0] = 0.0;
        let dead = LogPosteriors::new(only_blank, 1, vocab.len()).expect("matrix");
        assert_eq!(gop(&dead, cols, 0, 0..1), Err(GopError::NoEvidence(0)));
    }
}
