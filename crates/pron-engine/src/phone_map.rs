//! The ARPAbet to model-label map (ASSESSMENT_SPEC 7.1 step 2).
//!
//! The map is data (`data/phone_map.toml`), not code, so changing the phoneme
//! model means editing a file. Loading checks that all 39 symbols are covered;
//! binding to a vocabulary checks that every symbol still has a label the
//! model can actually output.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::arpabet::Arpabet;
use crate::vocab::ModelVocab;

const BUNDLED: &str = include_str!("../data/phone_map.toml");

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PhoneMapError {
    #[error("the phone map is not valid: {0}")]
    Toml(String),
    #[error("`{0}` is not an ARPAbet symbol")]
    UnknownSymbol(String),
    #[error("ARPAbet symbol {0} has no entry in the phone map")]
    Missing(Arpabet),
    #[error("ARPAbet symbol {0} lists no labels")]
    NoLabels(Arpabet),
    #[error("ARPAbet symbol {0} lists the label `{1}` twice")]
    DuplicateLabel(Arpabet, String),
    #[error("ARPAbet symbol {0} maps to none of the model's labels")]
    NoLabelInVocab(Arpabet),
    #[error("ARPAbet symbol {0} maps to the blank label")]
    MapsToBlank(Arpabet),
}

#[derive(Debug, Deserialize)]
struct PhoneMapFile {
    meta: Meta,
    map: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct Meta {
    model: String,
    status: String,
}

/// Every ARPAbet symbol with the model labels that count as that sound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhoneMap {
    /// The model the labels were written for.
    pub model: String,
    /// How far the labels have been checked against that model's vocabulary.
    pub status: String,
    labels: BTreeMap<Arpabet, Vec<String>>,
}

impl PhoneMap {
    /// The map that ships with the crate. It is a candidate for the model in the
    /// licence register and has not been checked against a vocabulary.
    pub fn bundled_candidate() -> Result<PhoneMap, PhoneMapError> {
        PhoneMap::from_toml(BUNDLED)
    }

    pub fn from_toml(text: &str) -> Result<PhoneMap, PhoneMapError> {
        let file: PhoneMapFile =
            toml::from_str(text).map_err(|e| PhoneMapError::Toml(e.to_string()))?;
        let mut labels = BTreeMap::new();
        for (key, list) in file.map {
            let symbol: Arpabet = key
                .parse()
                .map_err(|_| PhoneMapError::UnknownSymbol(key.clone()))?;
            if list.is_empty() {
                return Err(PhoneMapError::NoLabels(symbol));
            }
            for (i, label) in list.iter().enumerate() {
                if list[..i].contains(label) {
                    return Err(PhoneMapError::DuplicateLabel(symbol, label.clone()));
                }
            }
            labels.insert(symbol, list);
        }
        for symbol in Arpabet::ALL {
            if !labels.contains_key(&symbol) {
                return Err(PhoneMapError::Missing(symbol));
            }
        }
        Ok(PhoneMap {
            model: file.meta.model,
            status: file.meta.status,
            labels,
        })
    }

    /// The labels listed for `symbol`, in the order the file gives them.
    pub fn labels_of(&self, symbol: Arpabet) -> &[String] {
        self.labels.get(&symbol).map_or(&[], Vec::as_slice)
    }

    /// Resolves the map against a model's vocabulary. Fails if any symbol would
    /// have no label the model can output.
    pub fn bind(&self, vocab: &ModelVocab) -> Result<BoundPhoneMap, PhoneMapError> {
        let mut forward: BTreeMap<Arpabet, Vec<usize>> = BTreeMap::new();
        let mut reverse: BTreeMap<usize, Vec<Arpabet>> = BTreeMap::new();
        for symbol in Arpabet::ALL {
            let mut indices = Vec::new();
            for label in self.labels_of(symbol) {
                if let Some(index) = vocab.index_of(label) {
                    if index == vocab.blank() {
                        return Err(PhoneMapError::MapsToBlank(symbol));
                    }
                    indices.push(index);
                    reverse.entry(index).or_default().push(symbol);
                }
            }
            if indices.is_empty() {
                return Err(PhoneMapError::NoLabelInVocab(symbol));
            }
            forward.insert(symbol, indices);
        }
        Ok(BoundPhoneMap {
            forward,
            reverse,
            blank: vocab.blank(),
            vocab_len: vocab.len(),
        })
    }
}

/// A phone map resolved to output columns of one model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundPhoneMap {
    forward: BTreeMap<Arpabet, Vec<usize>>,
    reverse: BTreeMap<usize, Vec<Arpabet>>,
    blank: usize,
    vocab_len: usize,
}

impl BoundPhoneMap {
    /// Output columns that count as `symbol`. Never empty.
    pub fn columns_of(&self, symbol: Arpabet) -> &[usize] {
        self.forward.get(&symbol).map_or(&[], Vec::as_slice)
    }

    /// Every ARPAbet symbol a column can stand for. Empty for the blank and for
    /// labels no symbol uses (such as a word-boundary token).
    pub fn symbols_of_column(&self, column: usize) -> &[Arpabet] {
        self.reverse.get(&column).map_or(&[], Vec::as_slice)
    }

    pub fn blank(&self) -> usize {
        self.blank
    }

    /// Number of output columns of the model this map was bound to.
    pub fn vocab_len(&self) -> usize {
        self.vocab_len
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A vocabulary that holds the first label of every symbol plus a blank.
    fn vocab_with_first_labels(map: &PhoneMap) -> ModelVocab {
        let mut labels = vec!["<pad>".to_owned()];
        for symbol in Arpabet::ALL {
            let first = map.labels_of(symbol)[0].clone();
            if !labels.contains(&first) {
                labels.push(first);
            }
        }
        ModelVocab::from_labels(labels, "<pad>").expect("vocab")
    }

    #[test]
    fn the_bundled_map_covers_every_arpabet_symbol_with_at_least_one_label() {
        let map = PhoneMap::bundled_candidate().expect("bundled map loads");
        for symbol in Arpabet::ALL {
            assert!(
                !map.labels_of(symbol).is_empty(),
                "{symbol} has no model label"
            );
        }
        assert!(map.status.contains("unverified"));
    }

    #[test]
    fn the_bundled_map_binds_to_a_vocabulary_that_has_one_label_per_symbol() {
        let map = PhoneMap::bundled_candidate().expect("bundled map loads");
        let vocab = vocab_with_first_labels(&map);
        let bound = map.bind(&vocab).expect("binds");
        for symbol in Arpabet::ALL {
            let columns = bound.columns_of(symbol);
            assert!(!columns.is_empty(), "{symbol}");
            assert!(
                columns
                    .iter()
                    .all(|c| *c != bound.blank() && *c < vocab.len())
            );
            for c in columns {
                assert!(bound.symbols_of_column(*c).contains(&symbol));
            }
        }
        assert!(bound.symbols_of_column(bound.blank()).is_empty());
        assert_eq!(bound.vocab_len(), vocab.len());
    }

    #[test]
    fn binding_fails_loudly_when_a_symbol_has_no_label_in_the_vocabulary() {
        let map = PhoneMap::bundled_candidate().expect("bundled map loads");
        let vocab = ModelVocab::from_labels(vec!["<pad>".into(), "b".into()], "<pad>").unwrap();
        assert_eq!(
            map.bind(&vocab),
            Err(PhoneMapError::NoLabelInVocab(Arpabet::AA))
        );
    }

    #[test]
    fn binding_refuses_a_label_that_is_the_blank() {
        let map = PhoneMap::bundled_candidate().expect("bundled map loads");
        let mut labels = vec!["b".to_owned()];
        for symbol in Arpabet::ALL {
            let first = map.labels_of(symbol)[0].clone();
            if !labels.contains(&first) {
                labels.push(first);
            }
        }
        let vocab = ModelVocab::from_labels(labels, "b").unwrap();
        assert_eq!(
            map.bind(&vocab),
            Err(PhoneMapError::MapsToBlank(Arpabet::B))
        );
    }

    #[test]
    fn a_shared_label_reports_every_symbol_it_could_stand_for() {
        let map = PhoneMap::bundled_candidate().expect("bundled map loads");
        let mut labels = vec!["<pad>".to_owned()];
        for symbol in Arpabet::ALL {
            for label in map.labels_of(symbol) {
                if !labels.contains(label) {
                    labels.push(label.clone());
                }
            }
        }
        let vocab = ModelVocab::from_labels(labels, "<pad>").unwrap();
        let bound = map.bind(&vocab).expect("binds to the full inventory");
        let flap = vocab.index_of("ɾ").expect("flap is in the full inventory");
        assert_eq!(bound.symbols_of_column(flap), [Arpabet::D, Arpabet::T]);
    }

    #[test]
    fn loading_rejects_bad_maps() {
        let full: String = Arpabet::ALL
            .iter()
            .map(|a| format!("{} = [\"x{}\"]\n", a, a))
            .collect();
        let head = "[meta]\nmodel = \"m\"\nstatus = \"s\"\n[map]\n";
        assert!(PhoneMap::from_toml(&format!("{head}{full}")).is_ok());

        let missing = full.replace("ZH = [\"xZH\"]\n", "");
        assert_eq!(
            PhoneMap::from_toml(&format!("{head}{missing}")),
            Err(PhoneMapError::Missing(Arpabet::ZH))
        );
        let unknown = format!("{head}{full}QQ = [\"q\"]\n");
        assert_eq!(
            PhoneMap::from_toml(&unknown),
            Err(PhoneMapError::UnknownSymbol("QQ".to_owned()))
        );
        let empty = format!("{head}{}", full.replace("AA = [\"xAA\"]", "AA = []"));
        assert_eq!(
            PhoneMap::from_toml(&empty),
            Err(PhoneMapError::NoLabels(Arpabet::AA))
        );
        let dup = format!(
            "{head}{}",
            full.replace("AA = [\"xAA\"]", "AA = [\"a\", \"a\"]")
        );
        assert_eq!(
            PhoneMap::from_toml(&dup),
            Err(PhoneMapError::DuplicateLabel(Arpabet::AA, "a".to_owned()))
        );
        assert!(matches!(
            PhoneMap::from_toml("not toml ["),
            Err(PhoneMapError::Toml(_))
        ));
    }
}
