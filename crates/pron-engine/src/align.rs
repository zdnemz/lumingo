//! CTC forced alignment (ASSESSMENT_SPEC 7.1 step 4).
//!
//! The reference is a chain of words. Each word is a set of alternative
//! pronunciations, and each pronunciation is a chain of phones with a CTC blank
//! allowed after every phone. One Viterbi pass over that graph picks, for every
//! word, the pronunciation that best explains the audio *and* places the word
//! boundaries. Because all alternatives compete over the same frames, their
//! scores are comparable without any length normalisation.
//!
//! This is a pure function of the posterior matrix and the reference. It does
//! not know about models, thresholds or scores.

use std::ops::Range;

use crate::arpabet::Arpabet;
use crate::phone_map::BoundPhoneMap;
use crate::posteriors::LogPosteriors;

/// The alternative pronunciations of one word, as ARPAbet symbols.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordSpec {
    pub variants: Vec<Vec<Arpabet>>,
}

/// Frames `[start, end)` that the best path spends on one phone. Never empty:
/// a path cannot leave a phone without visiting it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlignedPhone {
    pub symbol: Arpabet,
    pub start: usize,
    pub end: usize,
}

impl AlignedPhone {
    pub fn frames(&self) -> Range<usize> {
        self.start..self.end
    }
}

/// The pronunciation chosen for one word and where its phones fall.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlignedWord {
    /// Index into the word's `variants`.
    pub variant: usize,
    pub phones: Vec<AlignedPhone>,
}

impl AlignedWord {
    /// First and last frame the word's phones cover, as `[start, end)`.
    pub fn span(&self) -> Option<Range<usize>> {
        Some(self.phones.first()?.start..self.phones.last()?.end)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Alignment {
    pub words: Vec<AlignedWord>,
    /// Sum of log posteriors along the best path.
    pub log_prob: f32,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AlignError {
    #[error("there are no words to align")]
    NothingToAlign,
    #[error("word {0} has no pronunciation or an empty one")]
    EmptyWord(usize),
    #[error(
        "the posterior matrix has {got} label columns but the phone map was bound to {expected}"
    )]
    LabelCountMismatch { expected: usize, got: usize },
    /// The audio is too short for the reference, or the posteriors rule out
    /// every path. This is an outcome of bad input, not a bug.
    #[error("no alignment exists over {frames} frames")]
    NoFeasiblePath { frames: usize },
    #[error("internal alignment inconsistency: {0}")]
    Internal(&'static str),
}

#[derive(Debug, Clone, Copy)]
enum Kind {
    Blank,
    Unit {
        word: usize,
        variant: usize,
        symbol: Arpabet,
    },
}

#[derive(Debug)]
struct Node {
    kind: Kind,
    /// Columns whose posterior can stand for this node (empty for a blank,
    /// which uses the blank column).
    columns: Vec<usize>,
    /// Nodes that may precede this one on the previous frame. Staying in the
    /// same node is always allowed and is not listed.
    preds: Vec<usize>,
}

fn disjoint(a: &[usize], b: &[usize]) -> bool {
    !a.iter().any(|x| b.contains(x))
}

/// The word graph: nodes, the nodes a path may start in, and the nodes it may
/// end in.
struct Graph {
    nodes: Vec<Node>,
    initial: Vec<usize>,
    finals: Vec<usize>,
}

fn build_graph(words: &[WordSpec], map: &BoundPhoneMap) -> Result<Graph, AlignError> {
    let mut nodes = vec![Node {
        kind: Kind::Blank,
        columns: Vec::new(),
        preds: Vec::new(),
    }];
    let mut initial = vec![0usize];
    // (blank node, phone node) at the end of each variant of the previous word.
    let mut prev_tails: Vec<(usize, usize)> = Vec::new();

    for (w, word) in words.iter().enumerate() {
        if word.variants.is_empty() || word.variants.iter().any(Vec::is_empty) {
            return Err(AlignError::EmptyWord(w));
        }
        let mut tails = Vec::with_capacity(word.variants.len());
        for (v, variant) in word.variants.iter().enumerate() {
            let mut last: Option<(usize, usize)> = None; // (blank node, phone node)
            for symbol in variant {
                let columns = map.columns_of(*symbol).to_vec();
                let mut preds = Vec::new();
                match last {
                    Some((blank, unit)) => {
                        preds.push(blank);
                        // Without a blank between them, two phones that the
                        // model writes with the same label would merge.
                        if disjoint(&nodes[unit].columns, &columns) {
                            preds.push(unit);
                        }
                    }
                    None if w == 0 => preds.push(0),
                    None => {
                        for (blank, unit) in &prev_tails {
                            preds.push(*blank);
                            if disjoint(&nodes[*unit].columns, &columns) {
                                preds.push(*unit);
                            }
                        }
                    }
                }
                let unit_index = nodes.len();
                if last.is_none() && w == 0 {
                    initial.push(unit_index);
                }
                nodes.push(Node {
                    kind: Kind::Unit {
                        word: w,
                        variant: v,
                        symbol: *symbol,
                    },
                    columns,
                    preds,
                });
                let blank_index = nodes.len();
                nodes.push(Node {
                    kind: Kind::Blank,
                    columns: Vec::new(),
                    preds: vec![unit_index],
                });
                last = Some((blank_index, unit_index));
            }
            if let Some(tail) = last {
                tails.push(tail);
            }
        }
        prev_tails = tails;
    }

    if prev_tails.is_empty() {
        return Err(AlignError::NothingToAlign);
    }
    let finals = prev_tails
        .iter()
        .flat_map(|(blank, unit)| [*unit, *blank])
        .collect();
    Ok(Graph {
        nodes,
        initial,
        finals,
    })
}

fn emission(node: &Node, row: &[f32], blank: usize) -> f32 {
    match node.kind {
        Kind::Blank => row[blank],
        Kind::Unit { .. } => node
            .columns
            .iter()
            .map(|c| row[*c])
            .fold(f32::NEG_INFINITY, f32::max),
    }
}

/// Aligns `words` to `posteriors`.
///
/// A phone's emission at a frame is the best log posterior among the labels
/// the phone map lists for it. Taking the best, not the sum, keeps GOP at most
/// zero and avoids counting one sound twice when a model has two spellings of it.
pub fn align(
    posteriors: &LogPosteriors,
    map: &BoundPhoneMap,
    words: &[WordSpec],
) -> Result<Alignment, AlignError> {
    if words.is_empty() {
        return Err(AlignError::NothingToAlign);
    }
    if posteriors.labels() != map.vocab_len() {
        return Err(AlignError::LabelCountMismatch {
            expected: map.vocab_len(),
            got: posteriors.labels(),
        });
    }
    let Graph {
        nodes,
        initial,
        finals,
    } = build_graph(words, map)?;
    let frames = posteriors.frames();
    if frames == 0 {
        return Err(AlignError::NoFeasiblePath { frames });
    }
    let n = nodes.len();
    let blank = map.blank();
    let row_at = |t: usize| {
        posteriors
            .row(t)
            .ok_or(AlignError::Internal("frame out of range"))
    };

    let mut prev = vec![f32::NEG_INFINITY; n];
    let mut cur = vec![f32::NEG_INFINITY; n];
    let mut back = vec![0u32; frames * n];

    let first = row_at(0)?;
    for &i in &initial {
        prev[i] = emission(&nodes[i], first, blank);
    }
    for t in 1..frames {
        let row = row_at(t)?;
        for (i, node) in nodes.iter().enumerate() {
            let mut best = prev[i];
            let mut from = i;
            for &p in &node.preds {
                if prev[p] > best {
                    best = prev[p];
                    from = p;
                }
            }
            cur[i] = if best == f32::NEG_INFINITY {
                f32::NEG_INFINITY
            } else {
                best + emission(node, row, blank)
            };
            // A node index always fits: the graph is far smaller than 2^32.
            back[t * n + i] =
                u32::try_from(from).map_err(|_| AlignError::Internal("graph too large"))?;
        }
        std::mem::swap(&mut prev, &mut cur);
    }

    let mut end_node = None;
    let mut best = f32::NEG_INFINITY;
    for &i in &finals {
        if prev[i] > best {
            best = prev[i];
            end_node = Some(i);
        }
    }
    let Some(mut node) = end_node else {
        return Err(AlignError::NoFeasiblePath { frames });
    };

    let mut path = vec![0usize; frames];
    path[frames - 1] = node;
    for t in (1..frames).rev() {
        node = back[t * n + node] as usize;
        path[t - 1] = node;
    }

    let mut per_word: Vec<Option<(usize, Vec<AlignedPhone>)>> = vec![None; words.len()];
    let mut t = 0;
    while t < frames {
        let node_index = path[t];
        let mut end = t + 1;
        while end < frames && path[end] == node_index {
            end += 1;
        }
        if let Kind::Unit {
            word,
            variant,
            symbol,
        } = nodes[node_index].kind
        {
            let entry = per_word[word].get_or_insert_with(|| (variant, Vec::new()));
            if entry.0 != variant {
                return Err(AlignError::Internal("a word switched pronunciation"));
            }
            entry.1.push(AlignedPhone {
                symbol,
                start: t,
                end,
            });
        }
        t = end;
    }

    let mut out = Vec::with_capacity(words.len());
    for (w, slot) in per_word.into_iter().enumerate() {
        let Some((variant, phones)) = slot else {
            return Err(AlignError::Internal("a word was skipped"));
        };
        if phones.len() != words[w].variants[variant].len() {
            return Err(AlignError::Internal(
                "a pronunciation was not fully visited",
            ));
        }
        out.push(AlignedWord { variant, phones });
    }
    Ok(Alignment {
        words: out,
        log_prob: best,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Synthetic, bound_map};
    use Arpabet::*;

    fn word(variants: &[&[Arpabet]]) -> WordSpec {
        WordSpec {
            variants: variants.iter().map(|v| v.to_vec()).collect(),
        }
    }

    fn spans(word: &AlignedWord) -> Vec<(Arpabet, usize, usize)> {
        word.phones
            .iter()
            .map(|p| (p.symbol, p.start, p.end))
            .collect()
    }

    #[test]
    fn one_word_known_spans() {
        let (map, vocab) = bound_map();
        let mut s = Synthetic::new(&vocab);
        s.blank(1)
            .phone(K, 2)
            .blank(1)
            .phone(AE, 3)
            .phone(T, 1)
            .blank(2);
        let m = s.matrix();
        let a = align(&m, &map, &[word(&[&[K, AE, T]])]).expect("aligns");
        assert_eq!(a.words.len(), 1);
        assert_eq!(a.words[0].variant, 0);
        assert_eq!(spans(&a.words[0]), [(K, 1, 3), (AE, 4, 7), (T, 7, 8)]);
        assert_eq!(a.words[0].span(), Some(1..8));
    }

    #[test]
    fn two_words_boundaries_follow_the_audio() {
        let (map, vocab) = bound_map();
        let mut s = Synthetic::new(&vocab);
        // "he said": HH IY | S EH D, with a pause between the words.
        s.phone(HH, 2)
            .phone(IY, 3)
            .blank(4)
            .phone(S, 3)
            .phone(EH, 2)
            .blank(1)
            .phone(D, 1);
        let m = s.matrix();
        let a = align(&m, &map, &[word(&[&[HH, IY]]), word(&[&[S, EH, D]])]).expect("aligns");
        assert_eq!(spans(&a.words[0]), [(HH, 0, 2), (IY, 2, 5)]);
        assert_eq!(spans(&a.words[1]), [(S, 9, 12), (EH, 12, 14), (D, 15, 16)]);
    }

    #[test]
    fn the_better_pronunciation_variant_wins_in_both_directions() {
        let (map, vocab) = bound_map();
        let read = word(&[&[R, EH, D], &[R, IY, D]]);

        let mut red = Synthetic::new(&vocab);
        red.phone(R, 2).phone(EH, 3).phone(D, 2);
        let a = align(&red.matrix(), &map, std::slice::from_ref(&read)).expect("aligns");
        assert_eq!(a.words[0].variant, 0);

        let mut reed = Synthetic::new(&vocab);
        reed.phone(R, 2).phone(IY, 3).phone(D, 2);
        let a = align(&reed.matrix(), &map, std::slice::from_ref(&read)).expect("aligns");
        assert_eq!(a.words[0].variant, 1);
        assert_eq!(spans(&a.words[0]), [(R, 0, 2), (IY, 2, 5), (D, 5, 7)]);
    }

    #[test]
    fn variants_of_different_length_compete_fairly() {
        let (map, vocab) = bound_map();
        // "to": T UW or just T. The audio says T UW, so the longer one must win
        // even though a shorter path pays for fewer phones.
        let to = word(&[&[T], &[T, UW]]);
        let mut long = Synthetic::new(&vocab);
        long.phone(T, 2).phone(UW, 4);
        let a = align(&long.matrix(), &map, std::slice::from_ref(&to)).expect("aligns");
        assert_eq!(a.words[0].variant, 1);

        let mut short = Synthetic::new(&vocab);
        short.phone(T, 3).blank(3);
        let a = align(&short.matrix(), &map, std::slice::from_ref(&to)).expect("aligns");
        assert_eq!(a.words[0].variant, 0);
    }

    #[test]
    fn a_variant_inside_a_longer_sentence_is_chosen_per_word() {
        let (map, vocab) = bound_map();
        let words = [
            word(&[&[DH, AH], &[DH, IY]]),
            word(&[&[K, AE, T]]),
            word(&[&[R, EH, D], &[R, IY, D]]),
        ];
        let mut s = Synthetic::new(&vocab);
        s.phone(DH, 2)
            .phone(IY, 2)
            .blank(1)
            .phone(K, 2)
            .phone(AE, 2)
            .phone(T, 2)
            .blank(2);
        s.phone(R, 2).phone(EH, 3).phone(D, 2);
        let a = align(&s.matrix(), &map, &words).expect("aligns");
        let chosen: Vec<usize> = a.words.iter().map(|w| w.variant).collect();
        assert_eq!(chosen, [1, 0, 0]);
    }

    #[test]
    fn identical_neighbours_need_a_blank_between_them() {
        let (map, vocab) = bound_map();
        // "hot time": T then T across the word boundary.
        let words = [word(&[&[HH, AA, T]]), word(&[&[T, AY, M]])];
        let mut s = Synthetic::new(&vocab);
        s.phone(HH, 1)
            .phone(AA, 2)
            .phone(T, 2)
            .blank(1)
            .phone(T, 2)
            .phone(AY, 2)
            .phone(M, 1);
        let a = align(&s.matrix(), &map, &words).expect("aligns");
        assert_eq!(spans(&a.words[0]), [(HH, 0, 1), (AA, 1, 3), (T, 3, 5)]);
        assert_eq!(spans(&a.words[1]), [(T, 6, 8), (AY, 8, 10), (M, 10, 11)]);
    }

    #[test]
    fn a_repeated_symbol_inside_one_word_is_split_by_the_blank() {
        let (map, vocab) = bound_map();
        let w = word(&[&[S, S]]);
        let mut s = Synthetic::new(&vocab);
        s.phone(S, 3).blank(2).phone(S, 2);
        let a = align(&s.matrix(), &map, &[w]).expect("aligns");
        assert_eq!(spans(&a.words[0]), [(S, 0, 3), (S, 5, 7)]);
    }

    #[test]
    fn tied_variants_resolve_to_the_first_listed() {
        let (map, vocab) = bound_map();
        let w = word(&[&[K, AE, T], &[K, AE, T]]);
        let mut s = Synthetic::new(&vocab);
        s.phone(K, 2).phone(AE, 2).phone(T, 2);
        let a = align(&s.matrix(), &map, &[w]).expect("aligns");
        assert_eq!(a.words[0].variant, 0);
    }

    #[test]
    fn too_few_frames_is_reported_not_forced() {
        let (map, vocab) = bound_map();
        let mut s = Synthetic::new(&vocab);
        s.phone(K, 1).phone(AE, 1);
        let err = align(&s.matrix(), &map, &[word(&[&[K, AE, T]])]).expect_err("too short");
        assert_eq!(err, AlignError::NoFeasiblePath { frames: 2 });
    }

    #[test]
    fn input_errors_are_typed() {
        let (map, vocab) = bound_map();
        let mut s = Synthetic::new(&vocab);
        s.phone(K, 3);
        let m = s.matrix();
        assert_eq!(align(&m, &map, &[]), Err(AlignError::NothingToAlign));
        assert_eq!(align(&m, &map, &[word(&[])]), Err(AlignError::EmptyWord(0)));
        assert_eq!(
            align(&m, &map, &[word(&[&[]])]),
            Err(AlignError::EmptyWord(0))
        );
        let narrow = LogPosteriors::new(vec![-1.0; 6], 2, 3).expect("matrix");
        assert!(matches!(
            align(&narrow, &map, &[word(&[&[K]])]),
            Err(AlignError::LabelCountMismatch { got: 3, .. })
        ));
        let empty = LogPosteriors::new(vec![], 0, vocab.len()).expect("matrix");
        assert_eq!(
            align(&empty, &map, &[word(&[&[K]])]),
            Err(AlignError::NoFeasiblePath { frames: 0 })
        );
    }

    #[test]
    fn random_sequences_are_recovered_exactly() {
        // The audio is built phone by phone, so the expected spans are known.
        // Several seeds stand in for property testing without a dependency.
        let (map, vocab) = bound_map();
        let pool = [K, AE, T, S, IY, D, N, OW, SH, UW, L, AY, M, B, F, EH, ER, G];
        for seed in 1..=40u64 {
            let mut rng = Lcg(seed);
            let word_count = 1 + rng.below(4);
            let mut words = Vec::new();
            let mut s = Synthetic::new(&vocab);
            let mut expected: Vec<Vec<(Arpabet, usize, usize)>> = Vec::new();
            let mut frame = 0usize;
            let lead = rng.below(3);
            s.blank(lead);
            frame += lead;
            let mut last: Option<Arpabet> = None;
            for _ in 0..word_count {
                let len = 1 + rng.below(5);
                let mut phones = Vec::new();
                let mut spans = Vec::new();
                for _ in 0..len {
                    let symbol = pool[rng.below(pool.len())];
                    // A blank is needed between equal neighbours; elsewhere it
                    // is optional.
                    let gap = if last == Some(symbol) {
                        1 + rng.below(2)
                    } else {
                        rng.below(2)
                    };
                    s.blank(gap);
                    frame += gap;
                    let dur = 1 + rng.below(4);
                    s.phone(symbol, dur);
                    spans.push((symbol, frame, frame + dur));
                    frame += dur;
                    phones.push(symbol);
                    last = Some(symbol);
                }
                words.push(WordSpec {
                    variants: vec![phones],
                });
                expected.push(spans);
            }
            s.blank(rng.below(3));
            let a = align(&s.matrix(), &map, &words).expect("aligns");
            for (w, exp) in expected.iter().enumerate() {
                assert_eq!(&spans(&a.words[w]), exp, "seed {seed}, word {w}");
            }
        }
    }

    struct Lcg(u64);
    impl Lcg {
        fn below(&mut self, n: usize) -> usize {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 33) as usize) % n
        }
    }
}
