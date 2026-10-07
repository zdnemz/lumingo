/// Splits a streamed reply into speakable sentences.
///
/// A sentence ends at '.', '!' or '?' followed by whitespace. Common
/// abbreviations ("Dr. Sari"), decimals ("3.5") and ellipses ("Well...") do not
/// end a sentence. A closing quote or bracket after the mark ("He said "no."
/// Then") keeps the sentence open until the next mark; that costs a longer
/// first chunk, never a wrong cut.
#[derive(Debug, Default)]
pub struct SentenceChunker {
    pending: String,
}

const ABBREVIATIONS: [&str; 6] = ["Mr.", "Mrs.", "Ms.", "Dr.", "e.g.", "i.e."];

impl SentenceChunker {
    /// Adds streamed text and returns every sentence that became complete.
    pub fn push(&mut self, delta: &str) -> Vec<String> {
        self.pending.push_str(delta);
        let mut out = Vec::new();
        while let Some(end) = self.first_sentence_end() {
            let rest = self.pending.split_off(end);
            let sentence = std::mem::replace(&mut self.pending, rest);
            let sentence = sentence.trim();
            if !sentence.is_empty() {
                out.push(sentence.to_owned());
            }
        }
        out
    }

    /// Call when the stream ends. Returns the remainder if it is not empty.
    pub fn finish(&mut self) -> Option<String> {
        let rest = std::mem::take(&mut self.pending);
        let rest = rest.trim();
        (!rest.is_empty()).then(|| rest.to_owned())
    }

    /// Byte index just after the first sentence-ending mark that is followed by
    /// whitespace, or `None` if no sentence is complete yet.
    fn first_sentence_end(&self) -> Option<usize> {
        let mut chars = self.pending.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            if !matches!(c, '.' | '!' | '?') {
                continue;
            }
            let end = i + c.len_utf8();
            let followed_by_space = matches!(chars.peek(), Some((_, next)) if next.is_whitespace());
            if !followed_by_space {
                continue;
            }
            if c == '.' {
                let before = &self.pending[..end];
                if before.ends_with("..") || ends_with_abbreviation(before) {
                    continue;
                }
            }
            return Some(end);
        }
        None
    }
}

/// True when `text` ends in an abbreviation that starts a word, so "Mdr." is not "Dr.".
fn ends_with_abbreviation(text: &str) -> bool {
    ABBREVIATIONS.iter().any(|abbr| {
        text.strip_suffix(abbr)
            .is_some_and(|head| !head.chars().next_back().is_some_and(char::is_alphanumeric))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunks(deltas: &[&str]) -> (Vec<String>, Option<String>) {
        let mut c = SentenceChunker::default();
        let out = deltas.iter().flat_map(|d| c.push(d)).collect();
        (out, c.finish())
    }

    #[test]
    fn splits_on_each_terminator() {
        let (s, rest) = chunks(&["Hello there. How are you? Fine! Ok"]);
        assert_eq!(s, ["Hello there.", "How are you?", "Fine!"]);
        assert_eq!(rest.as_deref(), Some("Ok"));
    }

    #[test]
    fn abbreviations_do_not_end_a_sentence() {
        let (s, _) = chunks(&["Dr. Sari is here. Mr. Budi too, e.g. today. Done. "]);
        assert_eq!(
            s,
            ["Dr. Sari is here.", "Mr. Budi too, e.g. today.", "Done."]
        );
    }

    #[test]
    fn an_abbreviation_inside_a_word_is_not_special() {
        let (s, _) = chunks(&["I met Mdr. Then I left. "]);
        assert_eq!(s, ["I met Mdr.", "Then I left."]);
    }

    #[test]
    fn decimals_do_not_end_a_sentence() {
        let (s, _) = chunks(&["It costs 3.5 dollars. Fine. "]);
        assert_eq!(s, ["It costs 3.5 dollars.", "Fine."]);
    }

    #[test]
    fn ellipsis_does_not_end_a_sentence() {
        let (s, _) = chunks(&["Well... I think so. Yes. "]);
        assert_eq!(s, ["Well... I think so.", "Yes."]);
    }

    #[test]
    fn no_final_punctuation_comes_out_of_finish() {
        let (s, rest) = chunks(&["First one. And then no ending"]);
        assert_eq!(s, ["First one."]);
        assert_eq!(rest.as_deref(), Some("And then no ending"));
    }

    #[test]
    fn a_mark_at_the_end_of_a_delta_waits_for_what_follows() {
        let (s, _) = chunks(&["Dr.", " Sari came.", " Then", " we left. "]);
        assert_eq!(s, ["Dr. Sari came.", "Then we left."]);
        let (s, _) = chunks(&["Hi."]);
        assert!(s.is_empty());
    }

    #[test]
    fn any_split_of_the_stream_gives_the_same_sentences() {
        let text = "Dr. Sari paid 3.5 dollars. Well... maybe! Is it ok? Yes. ";
        let whole = chunks(&[text]).0;
        let bytes: Vec<String> = text.chars().map(String::from).collect();
        let refs: Vec<&str> = bytes.iter().map(String::as_str).collect();
        assert_eq!(chunks(&refs).0, whole);
    }

    #[test]
    fn empty_input_gives_nothing() {
        assert_eq!(chunks(&["", "  "]), (vec![], None));
    }
}
