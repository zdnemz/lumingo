//! The output side as the loop sees it: the playback queue of `audio-io`.
//!
//! A full audio session hands out a `PlaybackHandle` that survives device
//! changes. A playback-only run (typed input with speech output) uses a bare
//! `PlaybackQueue`. Both do the same few things, so the loop talks to this trait.

use std::time::Duration;

use audio_io::{EnqueueOutcome, PlaybackError, PlaybackHandle, PlaybackQueue};
use speech::PcmChunk;

pub trait PlaybackPort: Send + Sync {
    /// Queues a chunk behind everything queued before. All of it fits or none of it is queued.
    fn enqueue(&self, chunk: &PcmChunk) -> Result<EnqueueOutcome, PlaybackError>;
    /// Says that everything queued so far ends the reply.
    fn finish_turn(&self);
    /// Drops everything queued. The output goes quiet within one device period.
    fn stop(&self);
    /// True while queued audio has not been played or flushed.
    fn is_active(&self) -> bool;
    /// Interleaved device samples handed to the output stream as real audio, ever.
    fn samples_played(&self) -> u64;
    /// Audio waiting in the queue.
    fn queued(&self) -> Duration;
}

impl PlaybackPort for PlaybackHandle {
    fn enqueue(&self, chunk: &PcmChunk) -> Result<EnqueueOutcome, PlaybackError> {
        PlaybackHandle::enqueue(self, chunk)
    }
    fn finish_turn(&self) {
        PlaybackHandle::finish_turn(self);
    }
    fn stop(&self) {
        PlaybackHandle::stop(self);
    }
    fn is_active(&self) -> bool {
        PlaybackHandle::is_active(self)
    }
    fn samples_played(&self) -> u64 {
        self.stats().samples_played
    }
    fn queued(&self) -> Duration {
        PlaybackHandle::queued(self)
    }
}

impl PlaybackPort for PlaybackQueue {
    fn enqueue(&self, chunk: &PcmChunk) -> Result<EnqueueOutcome, PlaybackError> {
        PlaybackQueue::enqueue(self, chunk)
    }
    fn finish_turn(&self) {
        PlaybackQueue::finish_turn(self);
    }
    fn stop(&self) {
        PlaybackQueue::stop(self);
    }
    fn is_active(&self) -> bool {
        PlaybackQueue::is_active(self)
    }
    fn samples_played(&self) -> u64 {
        self.stats().samples_played
    }
    fn queued(&self) -> Duration {
        PlaybackQueue::queued(self)
    }
}
