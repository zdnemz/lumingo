//! Incremental server-sent-events parser.
//!
//! Handles what real servers do: events split across network chunks at any byte,
//! `\n`, `\r\n` and bare `\r` line ends, comment lines that start with `:` (used
//! as keep-alives), multi-line `data:` fields, and a final event that is not
//! followed by a blank line.

use crate::error::LlmError;

/// A single line or event longer than this is a protocol error, not a reason to
/// grow memory without bound.
const MAX_PENDING_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

#[derive(Debug, Default)]
pub(crate) struct SseParser {
    buf: Vec<u8>,
    event: Option<String>,
    data: Vec<String>,
    started: bool,
}

impl SseParser {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Feeds raw bytes and returns every event completed by them.
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Result<Vec<SseEvent>, LlmError> {
        self.buf.extend_from_slice(bytes);
        if !self.started {
            // Strip a UTF-8 byte order mark once, at the very start.
            if self.buf.len() >= 3 {
                if self.buf.starts_with(&[0xEF, 0xBB, 0xBF]) {
                    self.buf.drain(..3);
                }
                self.started = true;
            } else if !b"\xEF\xBB\xBF".starts_with(&self.buf) {
                self.started = true;
            }
        }

        let mut events = Vec::new();
        let mut consumed = 0;
        loop {
            let rest = &self.buf[consumed..];
            let Some((line_end, next)) = find_line_end(rest) else {
                break;
            };
            let line = String::from_utf8_lossy(&rest[..line_end]).into_owned();
            consumed += next;
            if let Some(event) = self.handle_line(&line) {
                events.push(event);
            }
        }
        self.buf.drain(..consumed);

        let pending = self.buf.len() + self.data.iter().map(String::len).sum::<usize>();
        if pending > MAX_PENDING_BYTES {
            return Err(LlmError::Protocol(
                "a server-sent event was larger than 1 MiB".to_owned(),
            ));
        }
        Ok(events)
    }

    /// Call when the connection closes. A last event without its blank line is
    /// delivered rather than dropped, because some servers close right after the
    /// final `data:` line.
    pub(crate) fn finish(&mut self) -> Option<SseEvent> {
        if !self.buf.is_empty() {
            let line = String::from_utf8_lossy(&self.buf).into_owned();
            self.buf.clear();
            if let Some(event) = self.handle_line(line.trim_end_matches(['\r', '\n'])) {
                return Some(event);
            }
        }
        self.dispatch()
    }

    fn handle_line(&mut self, line: &str) -> Option<SseEvent> {
        if line.is_empty() {
            return self.dispatch();
        }
        if line.starts_with(':') {
            return None;
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match field {
            "event" => self.event = Some(value.to_owned()),
            "data" => self.data.push(value.to_owned()),
            // `id` and `retry` are reconnection hints; this client never reconnects a stream.
            _ => {}
        }
        None
    }

    fn dispatch(&mut self) -> Option<SseEvent> {
        let event = self.event.take();
        if self.data.is_empty() {
            return None;
        }
        let data = self.data.join("\n");
        self.data.clear();
        Some(SseEvent { event, data })
    }
}

/// Finds the end of the first line. Returns the length of the line without its
/// terminator and the offset of the next line. A trailing `\r` is held back because
/// the `\n` of a `\r\n` pair may arrive in the next chunk.
fn find_line_end(bytes: &[u8]) -> Option<(usize, usize)> {
    let position = bytes.iter().position(|b| *b == b'\n' || *b == b'\r')?;
    if bytes[position] == b'\n' {
        return Some((position, position + 1));
    }
    match bytes.get(position + 1) {
        Some(b'\n') => Some((position, position + 2)),
        Some(_) => Some((position, position + 1)),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_all(chunks: &[&[u8]]) -> Vec<SseEvent> {
        let mut parser = SseParser::new();
        let mut events = Vec::new();
        for chunk in chunks {
            events.extend(parser.push(chunk).expect("push"));
        }
        events.extend(parser.finish());
        events
    }

    #[test]
    fn parses_named_events_and_comments() {
        let events = parse_all(&[b": keep-alive\n\nevent: ping\ndata: {\"a\":1}\n\ndata: x\n\n"]);
        assert_eq!(
            events,
            vec![
                SseEvent {
                    event: Some("ping".into()),
                    data: "{\"a\":1}".into()
                },
                SseEvent {
                    event: None,
                    data: "x".into()
                },
            ]
        );
    }

    #[test]
    fn same_result_for_every_split_point() {
        let stream = b": hi\r\ndata: one\r\n\r\nevent: e\r\ndata: a\r\ndata: b\r\n\r\ndata: tail";
        let whole = parse_all(&[stream]);
        assert_eq!(whole.len(), 3);
        assert_eq!(whole[1].data, "a\nb");
        for split in 0..stream.len() {
            let (a, b) = stream.split_at(split);
            assert_eq!(parse_all(&[a, b]), whole, "split at {split}");
        }
    }

    #[test]
    fn bare_carriage_return_ends_a_line() {
        let events = parse_all(&[b"data: a\r\rdata: b\r\r"]);
        assert_eq!(
            events.iter().map(|e| e.data.as_str()).collect::<Vec<_>>(),
            ["a", "b"]
        );
    }

    #[test]
    fn a_multibyte_character_split_across_chunks_survives() {
        let text = "data: caf\u{e9}\n\n".as_bytes();
        let split = text.iter().position(|b| *b == 0xC3).expect("multibyte") + 1;
        let events = parse_all(&[&text[..split], &text[split..]]);
        assert_eq!(events[0].data, "caf\u{e9}");
    }

    #[test]
    fn strips_a_byte_order_mark_and_one_space_only() {
        let events = parse_all(&[b"\xEF\xBB\xBFdata:  two spaces\n\n"]);
        assert_eq!(events[0].data, " two spaces");
    }

    #[test]
    fn an_event_without_data_is_not_delivered() {
        assert!(parse_all(&[b"event: ping\n\n"]).is_empty());
    }

    #[test]
    fn oversized_input_is_a_protocol_error() {
        let mut parser = SseParser::new();
        let big = vec![b'a'; MAX_PENDING_BYTES + 1];
        assert!(matches!(parser.push(&big), Err(LlmError::Protocol(_))));
    }
}
