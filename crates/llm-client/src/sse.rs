/// One server-sent event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

/// Incremental decoder for `text/event-stream`. Bytes may arrive split anywhere,
/// including inside a line or a multi-byte character. Comment lines (keep-alives
/// that start with `:`) are dropped.
#[derive(Debug, Default)]
pub struct SseDecoder {
    line: Vec<u8>,
    event: Option<String>,
    data: Vec<String>,
}

impl SseDecoder {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<SseEvent> {
        let mut out = Vec::new();
        for &b in bytes {
            if b == b'\n' {
                self.end_of_line(&mut out);
            } else {
                self.line.push(b);
            }
        }
        out
    }

    /// Call when the connection closes. A stream that ends without a blank line
    /// (or without `[DONE]`) still delivers its last event.
    pub fn finish(&mut self) -> Vec<SseEvent> {
        let mut out = Vec::new();
        if !self.line.is_empty() {
            self.end_of_line(&mut out);
        }
        self.dispatch(&mut out);
        out
    }

    fn end_of_line(&mut self, out: &mut Vec<SseEvent>) {
        let raw = std::mem::take(&mut self.line);
        let line = String::from_utf8_lossy(&raw);
        let line = line.strip_suffix('\r').unwrap_or(&line);
        if line.is_empty() {
            self.dispatch(out);
        } else if line.starts_with(':') {
            // keep-alive
        } else {
            let (field, value) = line.split_once(':').unwrap_or((line, ""));
            let value = value.strip_prefix(' ').unwrap_or(value);
            match field {
                "event" => self.event = Some(value.to_owned()),
                "data" => self.data.push(value.to_owned()),
                _ => {}
            }
        }
    }

    fn dispatch(&mut self, out: &mut Vec<SseEvent>) {
        if self.data.is_empty() {
            self.event = None;
            return;
        }
        out.push(SseEvent {
            event: self.event.take(),
            data: self.data.drain(..).collect::<Vec<_>>().join("\n"),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(parts: &[&[u8]]) -> Vec<SseEvent> {
        let mut d = SseDecoder::default();
        let mut out: Vec<SseEvent> = parts.iter().flat_map(|p| d.push(p)).collect();
        out.extend(d.finish());
        out
    }

    fn ev(event: Option<&str>, data: &str) -> SseEvent {
        SseEvent {
            event: event.map(str::to_owned),
            data: data.to_owned(),
        }
    }

    #[test]
    fn parses_data_and_named_events_and_skips_keep_alives() {
        let out = decode(&[b": ping\n\ndata: one\n\nevent: x\ndata: two\n\n"]);
        assert_eq!(out, [ev(None, "one"), ev(Some("x"), "two")]);
    }

    #[test]
    fn handles_crlf_and_multi_line_data() {
        assert_eq!(decode(&[b"data: a\r\ndata: b\r\n\r\n"]), [ev(None, "a\nb")]);
    }

    #[test]
    fn splits_anywhere_including_inside_a_utf8_character() {
        let whole = "data: caf\u{e9} \u{1f600}\n\n".as_bytes();
        let one_by_one: Vec<&[u8]> = whole.chunks(1).collect();
        assert_eq!(decode(&one_by_one), [ev(None, "caf\u{e9} \u{1f600}")]);
    }

    #[test]
    fn a_stream_that_closes_without_a_blank_line_still_delivers_its_last_event() {
        assert_eq!(decode(&[b"data: last"]), [ev(None, "last")]);
        assert_eq!(decode(&[b"data: last\n"]), [ev(None, "last")]);
    }

    #[test]
    fn an_event_name_without_data_does_not_leak_into_the_next_event() {
        assert_eq!(decode(&[b"event: lonely\n\ndata: x\n\n"]), [ev(None, "x")]);
    }
}
