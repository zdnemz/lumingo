//! Capture, playback, resampling, the microphone gate, device handling and
//! session lifecycle for Lumingo.
//!
//! Everything here is plain logic that runs against fake devices in tests. The
//! real `cpal` backend is behind the off-by-default `cpal-backend` feature.
#![forbid(unsafe_code)]

mod capture;
mod format;
mod gate;
mod playback;
mod resample;
mod ring;
mod sync;

pub use capture::{CapturePath, Pumped};
pub use format::StreamFormat;
pub use gate::{
    GateCounters, GateStats, HOLD_AFTER_PLAYBACK, MicGate, PlaybackActivity, PushToTalk, Route,
};
pub use playback::{
    EnqueueOutcome, PlaybackError, PlaybackQueue, PlaybackSource, PlaybackStats, playback_queue,
};

pub use resample::{CaptureConverter, MonoResampler, ResampleError};

pub use ring::{CaptureConsumer, CaptureCounters, CaptureProducer, CaptureSnapshot, capture_ring};
