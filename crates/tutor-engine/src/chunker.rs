/// Splits a streamed reply into speakable sentences, so the first sentence can
/// be spoken while the rest is still arriving.
///
/// A sentence ends at `.`, `!` or `?` followed by optional closing quotes or
/// brackets and then whitespace. These do not end a sentence: common
/// abbreviations ("Dr. Sari"), decimals ("3.5"), and an ellipsis ("Well... I
/// think so").
#[derive(Debug, Default)]
pub struct SentenceChunker {
    pending: String,
}

/// Abbreviations whose full stop does not end a sentence. Matched case-sensitively at the end of the text so far.
const ABBREVIATIONS: [&str; 12] = [
    "Mr.", "Mrs.", "Ms.", "Dr.", "Prof.", "St.", "Mt.", "vs.", "e.g.", "i.e.", "U.S.", "a.m.",
];

/// Characters that may close a sentence after its final mark.
fn is_closer(c: char) -> bool {
    matches!(c, '"' | '\'' | ')' | ']' | '\u{201d}' | '\u{2019}')
}

impl SentenceChunker {
    pub fn new() -> Self {
        Self::default()
    }

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

    /// Byte index just after the first sentence end, or `None` if no sentence is
    /// complete yet. The whitespace after a mark must already have arrived, so
    /// "3." followed later by "5" is never cut early.
    fn first_sentence_end(&self) -> Option<usize> {
        let text = self.pending.as_str();
        let mut chars = text.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            if !matches!(c, '.' | '!' | '?') {
                continue;
            }
            // Part of an ellipsis: the next or the previous character is also a full stop.
            if c == '.' {
                let next_is_dot = matches!(chars.peek(), Some((_, '.')));
                let previous_is_dot = text[..i].ends_with('.');
                if next_is_dot || previous_is_dot {
                    continue;
                }
            }
            let mut end = i + c.len_utf8();
            // Skip closing quotes and brackets that belong to this sentence.
            let mut lookahead = text[end..].chars();
            let mut closers = 0;
            let after = loop {
                match lookahead.next() {
                    Some(next) if is_closer(next) => closers += next.len_utf8(),
                    other => break other,
                }
            };
            match after {
                Some(next) if next.is_whitespace() => {}
                _ => continue,
            }
            end += closers;
            if c == '.'
                && ABBREVIATIONS
                    .iter()
                    .any(|abbr| text[..i + 1].ends_with(abbr))
            {
                continue;
            }
            return Some(end);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::SentenceChunker;

    fn run(parts: &[&str]) -> (Vec<String>, Option<String>) {
        let mut chunker = SentenceChunker::new();
        let mut sentences = Vec::new();
        for part in parts {
            sentences.extend(chunker.push(part));
        }
        (sentences, chunker.finish())
    }

    #[test]
    fn splits_at_sentence_marks_followed_by_space() {
        let (sentences, rest) = run(&["Hello there. How are you? I am fine! "]);
        assert_eq!(sentences, ["Hello there.", "How are you?", "I am fine!"]);
        assert_eq!(rest, None);
    }

    #[test]
    fn waits_for_the_space_after_the_mark_before_cutting() {
        let mut chunker = SentenceChunker::new();
        assert!(chunker.push("It costs 3.").is_empty());
        assert!(chunker.push("5 dollars.").is_empty());
        assert_eq!(chunker.push(" Next"), ["It costs 3.5 dollars."]);
        assert_eq!(chunker.finish().as_deref(), Some("Next"));
    }

    #[test]
    fn a_decimal_does_not_end_a_sentence() {
        let (sentences, rest) = run(&["It is 3.5 metres long. Good. "]);
        assert_eq!(sentences, ["It is 3.5 metres long.", "Good."]);
        assert_eq!(rest, None);
    }

    #[test]
    fn abbreviations_do_not_end_a_sentence() {
        let (sentences, _) =
            run(&["Dr. Sari is here. Mr. Adi and Mrs. Lee, e.g. my teachers, came too. "]);
        assert_eq!(
            sentences,
            [
                "Dr. Sari is here.",
                "Mr. Adi and Mrs. Lee, e.g. my teachers, came too."
            ]
        );
    }

    #[test]
    fn an_ellipsis_does_not_end_a_sentence() {
        let (sentences, rest) = run(&["Well... I think so. Maybe... no. "]);
        assert_eq!(sentences, ["Well... I think so.", "Maybe... no."]);
        assert_eq!(rest, None);
    }

    #[test]
    fn closing_quotes_and_brackets_stay_with_their_sentence() {
        let (sentences, _) = run(&["She said \"Hello.\" Then she left (quickly!) and ran. "]);
        assert_eq!(
            sentences,
            [
                "She said \"Hello.\"",
                "Then she left (quickly!)",
                "and ran."
            ]
        );
    }

    #[test]
    fn text_without_final_punctuation_comes_out_at_finish() {
        let (sentences, rest) = run(&["First one. And the last part"]);
        assert_eq!(sentences, ["First one."]);
        assert_eq!(rest.as_deref(), Some("And the last part"));
    }

    #[test]
    fn works_when_the_stream_arrives_in_tiny_pieces() {
        let text = "Hi! This is Dr. Sari. Ready?";
        let mut chunker = SentenceChunker::new();
        let mut out = Vec::new();
        for c in text.chars() {
            out.extend(chunker.push(&c.to_string()));
        }
        out.extend(chunker.finish());
        assert_eq!(out, ["Hi!", "This is Dr. Sari.", "Ready?"]);
    }

    #[test]
    fn empty_and_blank_input_yield_nothing() {
        let mut chunker = SentenceChunker::new();
        assert!(chunker.push("").is_empty());
        assert!(chunker.push("   \n ").is_empty());
        assert_eq!(chunker.finish(), None);
    }

    #[test]
    fn handles_multibyte_text_without_cutting_inside_a_character() {
        let (sentences, rest) = run(&["Café au lait. Naïve café? Yes. "]);
        assert_eq!(sentences, ["Café au lait.", "Naïve café?", "Yes."]);
        assert_eq!(rest, None);
    }
}
