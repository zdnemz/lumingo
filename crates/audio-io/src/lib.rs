//! Capture, playback, resampling, the microphone gate, device handling and
//! session lifecycle for Lumingo.
//!
//! Everything here is plain logic that runs against fake devices in tests. The
//! real `cpal` backend is behind the off-by-default `cpal-backend` feature.
#![forbid(unsafe_code)]

mod capture;
mod device;
#[cfg(any(test, feature = "test-support"))]
pub mod fake;
mod format;
mod gate;
mod playback;
mod resample;
mod ring;
mod session;
mod sync;

pub use capture::{CapturePath, Pumped};
pub use device::{
    AudioBackend, AudioStream, Choice, DeviceError, DeviceId, DeviceInfo, DevicePrefs, DeviceRef,
    DeviceRegistry, Direction, FilePrefs, InputCallback, MemoryPrefs, OutputCallback, PrefsError,
    PrefsStore, ResolvedDevice, StreamErrorCallback,
};
pub use format::StreamFormat;
pub use gate::{
    GateCounters, GateStats, HOLD_AFTER_PLAYBACK, MicGate, PlaybackActivity, PushToTalk, Route,
};
pub use playback::{
    EnqueueOutcome, PlaybackError, PlaybackQueue, PlaybackSource, PlaybackStats, playback_queue,
};

pub use resample::{CaptureConverter, MonoResampler, ResampleError};

pub use ring::{CaptureConsumer, CaptureCounters, CaptureProducer, CaptureSnapshot, capture_ring};
pub use session::{
    AudioEvent, AudioSession, FrameSink, PlaybackHandle, SessionConfig, SessionError, SessionStats,
    StopReport,
};
