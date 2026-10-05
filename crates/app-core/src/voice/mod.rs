//! The voice loop: listen, transcribe, think, speak (ROADMAP S3-06).
//!
//! The orchestration lives here, not in a binary, so `tools/tutor-cli` and later
//! `apps/server` (through [`crate::SessionService`]) run the same code. The loop
//! is wired over trait objects only: `audio_io::AudioBackend` through a
//! `DeviceRegistry`, `speech::Vad` with the endpointer, the STT and TTS workers
//! over `SttEngine` and `TtsEngine`, and `Arc<dyn LlmClient>`.
//!
//! # Path of one spoken turn
//!
//! ```text
//! microphone -> capture ring -> 16 kHz frames -> microphone gate
//!   -> frame queue -> [vad thread] VAD + endpointer -> inbox
//!   -> [stt thread] transcript -> inbox
//!   -> [Tokio task] T1 prompt -> LLM stream -> sentence chunker
//!   -> [tts thread] synthesis -> playback queue -> speakers
//! ```
//!
//! The session phase is the tutor engine's own state machine (`Listening`,
//! `Transcribing`, `Thinking`, `Speaking`, `Paused`, `ProviderUnavailable`,
//! `EngineError`); [`phase`] only wraps it.
//!
//! # Threads and tasks
//!
//! | Where | What | Blocking allowed |
//! |---|---|---|
//! | audio callbacks (backend) | copy samples | no |
//! | `lumingo-capture` (audio-io) | resample, frame, gate, call the frame sink | no |
//! | `lumingo-vad` | the VAD model and the segmenter | yes, it is a thread of its own |
//! | `stt`, `tts` (speech workers) | one model each | yes |
//! | the orchestrator (one Tokio task) | the inbox, commands, session phase | never |
//! | one Tokio task per turn | LLM stream, chunker, TTS hand-over | never |
//! | recorder (one Tokio task) | database writes, background analysis | async only |
//!
//! Joining a worker (at the end of a session) runs on `spawn_blocking`.
//!
//! # Queues and what happens when one is full
//!
//! | Queue | Capacity | When full |
//! |---|---|---|
//! | capture ring (audio-io) | 2 s | the callback drops the block and counts it |
//! | frame queue, capture worker to VAD thread | 256 frames (8.2 s) | the new frame is dropped and counted (`frames_dropped`); the capture worker never waits |
//! | inbox, every producer to the orchestrator | 64 | threads drop the message and count it (`inbox_dropped`); async senders wait |
//! | STT input (speech) | 4 utterances | `submit` returns `Full`; the turn ends as a recoverable `EngineError` |
//! | utterance backlog (orchestrator) | 4 | the new utterance is dropped and counted (`utterances_dropped`) |
//! | audio session events (audio-io) | 32 | the new event is dropped and counted; the orchestrator reads them every 250 ms |
//! | TTS input (speech) | 8 sentences | `speak` returns the sentence; the turn keeps it and offers it again every poll; nothing is dropped |
//! | playback queue (audio-io) | 30 s | the chunk is handed back; the TTS worker retries every 5 ms and waits |
//! | notes of one turn (speaker to turn) | 32 | the note is dropped and counted (`notes_dropped`); the 5 s TTS limit then ends the turn |
//! | events (broadcast) | 64 | the oldest are dropped for a slow receiver, which is told how many |
//! | recorder | 16 | the turn is not recorded and counted (`recorder_dropped`) |
//!
//! # Latency
//!
//! Every turn records the five parts of `context_pack.md` section 4 (see
//! [`latency`]) from the last sample the VAD classified as speech to the first
//! tutor sample the output callback consumed, read from the injected
//! [`LoopClock`].
//!
//! # Interrupting
//!
//! The half-duplex gate of `audio-io` drops microphone frames while tutor audio
//! plays and for 150 ms after, so the learner cannot interrupt by voice while the
//! tutor is speaking. An utterance that still ends while the tutor speaks, in a gap
//! between two sentences, is dropped and counted, because it is most likely the
//! tutor's own voice. Stopping works three ways: [`VoiceHandle::stop_speaking`],
//! push-to-talk (frames that bypass the gate), and speaking while the tutor is
//! still thinking. Each one cancels the turn's LLM stream and TTS turn, stops the
//! playback queue and returns the session to `Listening`.
//!
//! # Cancellation
//!
//! A session has one `CancellationToken`; every turn has a child of it, and the
//! TTS turn and STT job have their own flags. Cancelling a turn reaches the LLM
//! stream at its next poll, the TTS worker before its next sentence, and
//! playback within one device period.

mod clock;
pub mod config;
mod engine;
mod error;
mod event;
mod latency;
mod listen;
mod msg;
mod phase;
mod port;
mod record;
mod speaker;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
mod turn;

pub use clock::{LoopClock, SystemLoopClock};
pub use config::{Scenario, VoiceConfig};
pub use engine::{
    AudioMode, EngineInfos, ListenParts, SttLoad, TtsLoad, VoiceHandle, VoiceLoop, VoiceParts,
    VoiceStats, VoiceSummary,
};
pub use error::{VoiceError, VoiceResult};
pub use event::{StopCause, TurnOutcome, VoiceEvent};
pub use latency::{
    LatencyParts, LatencyRecord, LatencySummary, Quantiles, Stamps, TurnLatency, duration_ms,
    percentile,
};
pub use port::PlaybackPort;
pub use record::{RecordSummary, Recording};

#[cfg(test)]
mod tests;
