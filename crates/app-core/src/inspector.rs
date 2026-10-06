//! The payload inspector: what was sent to the provider and what came back.
//!
//! The log is `llm_client::PayloadLog`, a bounded ring that every provider client
//! of this program writes to. See its module documentation for the capacity, the
//! cut at 16 KiB, and how the key is removed when an entry is captured. This file
//! only turns it into the API type.

use llm_client::{PayloadEntry, PayloadOutcome};

use crate::api::{InspectorReport, PayloadEntryView, PayloadOutcomeView};
use crate::core::AppCore;

fn outcome_view(outcome: PayloadOutcome) -> PayloadOutcomeView {
    match outcome {
        PayloadOutcome::Pending => PayloadOutcomeView::Pending,
        PayloadOutcome::Ok => PayloadOutcomeView::Ok,
        PayloadOutcome::HttpError => PayloadOutcomeView::HttpError,
        PayloadOutcome::Failed => PayloadOutcomeView::Failed,
        PayloadOutcome::Abandoned => PayloadOutcomeView::Abandoned,
    }
}

fn entry_view(entry: PayloadEntry) -> PayloadEntryView {
    PayloadEntryView {
        id: entry.id,
        started_unix_ms: entry.started_unix_ms,
        endpoint: entry.endpoint,
        streaming: entry.streaming,
        request_body: entry.request_body,
        request_truncated: entry.request_truncated,
        status: entry.status,
        response_body: entry.response_body,
        response_truncated: entry.response_truncated,
        outcome: outcome_view(entry.outcome),
        elapsed_ms: entry.elapsed_ms,
    }
}

impl AppCore {
    /// The last provider requests and responses, oldest first, with the limits.
    pub fn inspector(&self) -> InspectorReport {
        let snapshot = self.payload_log.snapshot();
        InspectorReport {
            capacity: u32::try_from(snapshot.capacity).unwrap_or(u32::MAX),
            max_body_bytes: u32::try_from(snapshot.max_body_bytes).unwrap_or(u32::MAX),
            dropped: snapshot.dropped,
            entries: snapshot.entries.into_iter().map(entry_view).collect(),
        }
    }
}
