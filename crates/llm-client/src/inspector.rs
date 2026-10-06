//! The payload inspector's log: the last provider requests and responses, kept
//! in memory so the learner can see exactly what left the machine.
//!
//! # What is kept
//!
//! One entry per HTTP request that was sent. A retry is its own entry, so a
//! retry that the learner never sees is visible here. An entry holds the
//! endpoint (host and path, without a query string or credentials), the JSON
//! request body, the HTTP status and the response body.
//!
//! # What is never kept
//!
//! * Headers. The key travels only in a header, and no header is stored.
//! * The key itself. Every body is scrubbed when it is captured, before it is
//!   written to the ring: the exact key is replaced, and so is any token that has
//!   the shape of a key (see [`crate::redact::scrub`]). A key that a learner pasted
//!   into a chat message therefore does not appear either.
//!
//! # Bounds and what is dropped
//!
//! * At most [`DEFAULT_CAPACITY`] entries (50) unless the owner of the log chose
//!   another number. When the ring is full the oldest entry is dropped and
//!   counted in [`PayloadSnapshot::dropped`]. Nothing waits.
//! * A request or response body is cut at [`MAX_BODY_BYTES`] (16 KiB). The cut is
//!   made on a character boundary and the entry says it was cut. The bytes after
//!   the cut are not kept anywhere.
//! * A streamed reply is recorded as the raw event-stream text (the `data:` lines)
//!   and is cut the same way.
//! * The log lives in memory. It is gone when the program stops.
//!
//! The log holds learner text (it is what was sent to the provider), so the
//! caller shows it only to the learner on the loopback address, and it is not
//! part of any export.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use reqwest::Url;
use serde::Serialize;

use crate::key::ApiKey;
use crate::redact::scrub;

/// Entries kept unless the creator of the log chose another number.
pub const DEFAULT_CAPACITY: usize = 50;

/// Longest request or response body kept, in bytes.
pub const MAX_BODY_BYTES: usize = 16 * 1024;

/// How a captured request ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PayloadOutcome {
    /// Sent, and no answer has been fully read yet.
    Pending,
    /// The provider answered with a success status and the body was read.
    Ok,
    /// The provider answered with an error status.
    HttpError,
    /// No answer: the connection failed, timed out or the call was cancelled.
    Failed,
    /// A streamed reply that the caller stopped reading before its end.
    Abandoned,
}

/// One request and what came back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PayloadEntry {
    /// Grows by one for every request sent, from 1. Not reused when an entry is
    /// dropped, so a gap in the numbers shows what was dropped.
    pub id: u64,
    pub started_unix_ms: u64,
    /// `host/path`, never a query string, port credentials or a key.
    pub endpoint: String,
    pub streaming: bool,
    pub request_body: String,
    pub request_truncated: bool,
    pub status: Option<u16>,
    pub response_body: Option<String>,
    pub response_truncated: bool,
    pub outcome: PayloadOutcome,
    pub elapsed_ms: Option<u64>,
}

/// The log as it is shown: the entries, newest last, and the limits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PayloadSnapshot {
    pub capacity: usize,
    pub max_body_bytes: usize,
    /// Entries dropped since the program started because the ring was full.
    pub dropped: u64,
    pub entries: Vec<PayloadEntry>,
}

#[derive(Debug)]
struct Ring {
    capacity: usize,
    next_id: u64,
    dropped: u64,
    entries: VecDeque<PayloadEntry>,
}

/// A bounded ring of captured provider traffic. Cloning shares the ring.
#[derive(Debug, Clone)]
pub struct PayloadLog {
    inner: Arc<Mutex<Ring>>,
}

impl Default for PayloadLog {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

fn lock(ring: &Mutex<Ring>) -> MutexGuard<'_, Ring> {
    // A panic elsewhere cannot leave the ring half written: every change is one
    // push or one field assignment.
    ring.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Cuts `text` to at most `limit` bytes on a character boundary.
fn cut(text: String, limit: usize) -> (String, bool) {
    if text.len() <= limit {
        return (text, false);
    }
    let mut end = limit;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_owned(), true)
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// `host/path` of a request URL. The query string, the fragment and any
/// credentials in the URL are left out, because they could carry a secret.
fn endpoint_of(url: &Url) -> String {
    let host = url.host_str().unwrap_or("unknown-host");
    let path = url.path().trim_end_matches('/');
    format!("{host}{path}")
}

impl PayloadLog {
    /// A log that keeps the newest `capacity` entries. A capacity of 0 keeps 1.
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Ring {
                capacity: capacity.max(1),
                next_id: 1,
                dropped: 0,
                entries: VecDeque::new(),
            })),
        }
    }

    /// The entries, oldest first, with the limits and the drop count.
    pub fn snapshot(&self) -> PayloadSnapshot {
        let ring = lock(&self.inner);
        PayloadSnapshot {
            capacity: ring.capacity,
            max_body_bytes: MAX_BODY_BYTES,
            dropped: ring.dropped,
            entries: ring.entries.iter().cloned().collect(),
        }
    }

    /// Forgets every entry. The drop count and the numbering go on.
    pub fn clear(&self) {
        lock(&self.inner).entries.clear();
    }

    /// Records a request that is about to be sent. `request` is the serialised
    /// JSON body. The returned handle completes the entry.
    pub(crate) fn begin(
        &self,
        url: &Url,
        request: &str,
        streaming: bool,
        key: Option<&ApiKey>,
    ) -> Capture {
        let secret = key.map(|k| k.expose().to_owned());
        let (body, truncated) = cut(scrub(request, secret.as_deref()), MAX_BODY_BYTES);
        let mut ring = lock(&self.inner);
        let id = ring.next_id;
        ring.next_id += 1;
        if ring.entries.len() >= ring.capacity {
            ring.entries.pop_front();
            ring.dropped += 1;
        }
        ring.entries.push_back(PayloadEntry {
            id,
            started_unix_ms: unix_ms(),
            endpoint: endpoint_of(url),
            streaming,
            request_body: body,
            request_truncated: truncated,
            status: None,
            response_body: None,
            response_truncated: false,
            outcome: PayloadOutcome::Pending,
            elapsed_ms: None,
        });
        drop(ring);
        Capture {
            log: self.clone(),
            id,
            started: Instant::now(),
            secret,
            status: None,
            buffer: Vec::new(),
            buffer_full: false,
            done: false,
        }
    }
}

/// The open end of one entry. It collects the response and writes it, scrubbed,
/// when the entry is finished. Dropping it unfinished marks the entry
/// [`PayloadOutcome::Abandoned`].
#[derive(Debug)]
pub(crate) struct Capture {
    log: PayloadLog,
    id: u64,
    started: Instant,
    secret: Option<String>,
    status: Option<u16>,
    /// Raw response bytes, at most `MAX_BODY_BYTES`. They are scrubbed as a
    /// whole when the entry is written, so a key split over two chunks is still
    /// found.
    buffer: Vec<u8>,
    buffer_full: bool,
    done: bool,
}

impl Capture {
    /// Notes the status line of the response.
    pub(crate) fn set_status(&mut self, status: u16) {
        self.status = Some(status);
    }

    /// Adds response bytes, up to the body limit.
    pub(crate) fn append(&mut self, bytes: &[u8]) {
        let room = MAX_BODY_BYTES.saturating_sub(self.buffer.len());
        if bytes.len() > room {
            self.buffer_full = true;
        }
        self.buffer
            .extend_from_slice(&bytes[..bytes.len().min(room)]);
    }

    /// Finishes the entry with what was collected and the outcome.
    pub(crate) fn finish(mut self, outcome: PayloadOutcome) {
        self.write(outcome);
    }

    /// Finishes the entry with a whole body that was read elsewhere.
    pub(crate) fn finish_with(mut self, outcome: PayloadOutcome, body: &[u8]) {
        self.append(body);
        self.write(outcome);
    }

    fn write(&mut self, outcome: PayloadOutcome) {
        if self.done {
            return;
        }
        self.done = true;
        let elapsed = u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let raw = String::from_utf8_lossy(&self.buffer).into_owned();
        let (response, cut_now) = cut(scrub(&raw, self.secret.as_deref()), MAX_BODY_BYTES);
        let has_body = !self.buffer.is_empty();
        let mut ring = lock(&self.log.inner);
        // The entry may have been dropped to make room for newer ones.
        if let Some(entry) = ring.entries.iter_mut().find(|e| e.id == self.id) {
            entry.status = self.status;
            entry.outcome = outcome;
            entry.elapsed_ms = Some(elapsed);
            if has_body {
                entry.response_body = Some(response);
                entry.response_truncated = self.buffer_full || cut_now;
            }
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.write(PayloadOutcome::Abandoned);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url() -> Url {
        Url::parse("https://api.example.test:8443/v1/chat/completions?api_key=abc&x=1")
            .expect("url")
    }

    fn key() -> ApiKey {
        ApiKey::new("sk-live-0123456789ABCDEFGHIJ").expect("key")
    }

    #[test]
    fn an_entry_has_host_and_path_only_and_the_ring_numbers_requests() {
        let log = PayloadLog::new(3);
        let first = log.begin(&url(), "{\"a\":1}", false, None);
        first.finish_with(PayloadOutcome::Ok, b"{\"b\":2}");
        let snap = log.snapshot();
        assert_eq!(snap.entries.len(), 1);
        let entry = &snap.entries[0];
        assert_eq!(entry.id, 1);
        assert_eq!(entry.endpoint, "api.example.test/v1/chat/completions");
        assert!(!entry.endpoint.contains("api_key"));
        assert_eq!(entry.request_body, "{\"a\":1}");
        assert_eq!(entry.response_body.as_deref(), Some("{\"b\":2}"));
        assert_eq!(entry.outcome, PayloadOutcome::Ok);
        assert!(entry.elapsed_ms.is_some());
    }

    #[test]
    fn the_oldest_entry_is_dropped_and_counted_when_the_ring_is_full() {
        let log = PayloadLog::new(2);
        for _ in 0..5 {
            log.begin(&url(), "{}", false, None)
                .finish(PayloadOutcome::Ok);
        }
        let snap = log.snapshot();
        assert_eq!(snap.capacity, 2);
        assert_eq!(snap.dropped, 3);
        let ids: Vec<u64> = snap.entries.iter().map(|e| e.id).collect();
        assert_eq!(
            ids,
            [4, 5],
            "numbers are not reused, so the gap shows the drop"
        );
    }

    #[test]
    fn a_capacity_of_zero_keeps_one_entry() {
        let log = PayloadLog::new(0);
        log.begin(&url(), "{}", false, None)
            .finish(PayloadOutcome::Ok);
        log.begin(&url(), "{}", false, None)
            .finish(PayloadOutcome::Ok);
        assert_eq!(log.snapshot().entries.len(), 1);
    }

    #[test]
    fn the_key_is_removed_from_both_bodies_even_when_split_over_chunks() {
        let log = PayloadLog::default();
        let key = key();
        let request = format!(
            "{{\"messages\":[{{\"content\":\"my key is {}\"}}]}}",
            "sk-live-0123456789ABCDEFGHIJ"
        );
        let mut capture = log.begin(&url(), &request, true, Some(&key));
        capture.set_status(200);
        capture.append(b"data: {\"echo\":\"sk-live-01234");
        capture.append(b"56789ABCDEFGHIJ\"}\n\n");
        capture.finish(PayloadOutcome::Ok);
        let snap = log.snapshot();
        let shown = format!("{snap:?}");
        assert!(!shown.contains("0123456789ABCDEFGHIJ"), "{shown}");
        assert!(!shown.contains("ABCDEFGHIJ"), "{shown}");
        assert!(snap.entries[0].request_body.contains("[redacted]"));
        assert!(
            snap.entries[0]
                .response_body
                .as_deref()
                .is_some_and(|b| b.contains("[redacted]"))
        );
        assert_eq!(snap.entries[0].status, Some(200));
    }

    #[test]
    fn a_long_body_is_cut_on_a_character_boundary_and_says_so() {
        let log = PayloadLog::default();
        // Each "é" is two bytes, so the limit falls inside one when the count is odd.
        let request = format!("\"{}\"", "é".repeat(MAX_BODY_BYTES));
        let mut capture = log.begin(&url(), &request, false, None);
        capture.append("é".repeat(MAX_BODY_BYTES).as_bytes());
        capture.finish(PayloadOutcome::Ok);
        let entry = &log.snapshot().entries[0];
        assert!(entry.request_truncated);
        assert!(entry.response_truncated);
        assert!(entry.request_body.len() <= MAX_BODY_BYTES);
        assert!(
            entry
                .response_body
                .as_deref()
                .is_some_and(|b| b.len() <= MAX_BODY_BYTES)
        );
        assert!(entry.request_body.chars().all(|c| c == 'é' || c == '"'));
    }

    #[test]
    fn a_capture_dropped_unfinished_is_marked_abandoned_with_what_arrived() {
        let log = PayloadLog::default();
        {
            let mut capture = log.begin(&url(), "{}", true, None);
            capture.set_status(200);
            capture.append(b"data: partial");
        }
        let entry = &log.snapshot().entries[0];
        assert_eq!(entry.outcome, PayloadOutcome::Abandoned);
        assert_eq!(entry.response_body.as_deref(), Some("data: partial"));
    }

    #[test]
    fn a_finished_entry_is_not_overwritten_by_the_drop() {
        let log = PayloadLog::default();
        let capture = log.begin(&url(), "{}", false, None);
        capture.finish(PayloadOutcome::Failed);
        assert_eq!(log.snapshot().entries[0].outcome, PayloadOutcome::Failed);
    }

    #[test]
    fn a_finished_capture_of_a_dropped_entry_does_nothing() {
        let log = PayloadLog::new(1);
        let old = log.begin(&url(), "{\"old\":1}", false, None);
        log.begin(&url(), "{\"new\":1}", false, None)
            .finish(PayloadOutcome::Ok);
        old.finish_with(PayloadOutcome::Ok, b"late");
        let snap = log.snapshot();
        assert_eq!(snap.entries.len(), 1);
        assert_eq!(snap.entries[0].request_body, "{\"new\":1}");
        assert!(snap.entries[0].response_body.is_none());
    }
}
