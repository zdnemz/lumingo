//! The label inventory of a phoneme recogniser, and which of its labels is the
//! CTC blank.

use std::collections::HashMap;

/// Why a vocabulary could not be built.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum VocabError {
    #[error("the vocabulary is not a JSON object of label to id: {0}")]
    NotAnObject(String),
    #[error("label ids are not a dense 0..{len} range (id {id} is missing or repeated)")]
    NotDense { len: usize, id: usize },
    #[error("the blank label `{0}` is not in the vocabulary")]
    BlankMissing(String),
    #[error("the vocabulary has no labels")]
    Empty,
    #[error("label `{0}` appears twice")]
    DuplicateLabel(String),
}

/// Label strings indexed by the model's output column, plus the blank column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelVocab {
    labels: Vec<String>,
    blank: usize,
}

impl ModelVocab {
    /// `labels[i]` is the label of output column `i`; `blank_label` names the
    /// CTC blank (for wav2vec2 checkpoints that is the pad token).
    pub fn from_labels(labels: Vec<String>, blank_label: &str) -> Result<ModelVocab, VocabError> {
        if labels.is_empty() {
            return Err(VocabError::Empty);
        }
        let mut seen = HashMap::new();
        for (i, label) in labels.iter().enumerate() {
            if seen.insert(label.as_str(), i).is_some() {
                return Err(VocabError::DuplicateLabel(label.clone()));
            }
        }
        let blank = seen
            .get(blank_label)
            .copied()
            .ok_or_else(|| VocabError::BlankMissing(blank_label.to_owned()))?;
        Ok(ModelVocab { labels, blank })
    }

    /// Reads a Hugging Face style `vocab.json`: `{"<pad>": 0, "ə": 1, ...}`.
    pub fn from_vocab_json(json: &str, blank_label: &str) -> Result<ModelVocab, VocabError> {
        let map: HashMap<String, usize> =
            serde_json::from_str(json).map_err(|e| VocabError::NotAnObject(e.to_string()))?;
        let len = map.len();
        let mut labels: Vec<Option<String>> = vec![None; len];
        for (label, id) in map {
            match labels.get_mut(id) {
                Some(slot @ None) => *slot = Some(label),
                _ => return Err(VocabError::NotDense { len, id }),
            }
        }
        let labels = labels
            .into_iter()
            .enumerate()
            .map(|(id, l)| l.ok_or(VocabError::NotDense { len, id }))
            .collect::<Result<Vec<_>, _>>()?;
        ModelVocab::from_labels(labels, blank_label)
    }

    pub fn len(&self) -> usize {
        self.labels.len()
    }

    pub fn is_empty(&self) -> bool {
        self.labels.is_empty()
    }

    /// Output column of the CTC blank.
    pub fn blank(&self) -> usize {
        self.blank
    }

    pub fn label(&self, index: usize) -> Option<&str> {
        self.labels.get(index).map(String::as_str)
    }

    pub fn index_of(&self, label: &str) -> Option<usize> {
        self.labels.iter().position(|l| l == label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_vocab_json_in_id_order() {
        let v = ModelVocab::from_vocab_json(r#"{"b":2,"<pad>":0,"a":1}"#, "<pad>").expect("ok");
        assert_eq!(v.len(), 3);
        assert_eq!(v.blank(), 0);
        assert_eq!(v.label(1), Some("a"));
        assert_eq!(v.index_of("b"), Some(2));
        assert_eq!(v.index_of("zz"), None);
    }

    #[test]
    fn rejects_gaps_repeats_and_a_missing_blank() {
        assert!(matches!(
            ModelVocab::from_vocab_json(r#"{"a":0,"b":2}"#, "a"),
            Err(VocabError::NotDense { .. })
        ));
        assert!(matches!(
            ModelVocab::from_vocab_json(r#"{"a":0,"b":0}"#, "a"),
            Err(VocabError::NotDense { .. })
        ));
        assert_eq!(
            ModelVocab::from_vocab_json(r#"{"a":0}"#, "<pad>"),
            Err(VocabError::BlankMissing("<pad>".to_owned()))
        );
        assert!(matches!(
            ModelVocab::from_vocab_json("[1,2]", "a"),
            Err(VocabError::NotAnObject(_))
        ));
        assert_eq!(ModelVocab::from_labels(vec![], "a"), Err(VocabError::Empty));
        assert_eq!(
            ModelVocab::from_labels(vec!["a".into(), "a".into()], "a"),
            Err(VocabError::DuplicateLabel("a".to_owned()))
        );
    }
}
