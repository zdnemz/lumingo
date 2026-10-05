//! A fake audio backend for tests.
//!
//! It exists only when the crate is compiled for its own tests or with the
//! `test-support` feature, which no release build enables. It behaves like a
//! device API: it lists devices, runs a thread per open stream that calls the
//! stream's callback once per period, lets a test unplug a device (the stream's
//! error callback fires with [`DeviceError::Lost`]) and counts the streams and
//! threads that are alive so lifecycle tests can check that none survive.

use std::f32::consts::PI;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::device::{
    AudioBackend, AudioStream, DeviceError, DeviceId, DeviceInfo, Direction, InputCallback,
    OutputCallback, StreamErrorCallback,
};
use crate::format::StreamFormat;
use crate::sync::lock;

/// Rendered output is kept for this many samples at most, so a long test cannot
/// grow without bound.
const RENDER_LIMIT: usize = 48_000 * 2 * 20;

struct OpenStream {
    device: DeviceId,
    name: String,
    direction: Direction,
    lost: Arc<AtomicBool>,
    on_error: StreamErrorCallback,
}

struct State {
    devices: Mutex<Vec<DeviceInfo>>,
    streams: Mutex<Vec<(usize, OpenStream)>>,
    next_stream: AtomicUsize,
    live_streams: AtomicUsize,
    live_threads: AtomicUsize,
    opened: AtomicUsize,
    input_tone: Mutex<Option<f32>>,
    rendered: Mutex<Vec<f32>>,
    period: Duration,
}

/// The fake. Clones share one set of devices and counters.
#[derive(Clone)]
pub struct FakeBackend {
    state: Arc<State>,
}

impl std::fmt::Debug for FakeBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeBackend").finish_non_exhaustive()
    }
}

impl Default for FakeBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeBackend {
    /// A backend whose streams call back every 10 ms, like a typical device.
    pub fn new() -> Self {
        Self::with_period(Duration::from_millis(10))
    }

    pub fn with_period(period: Duration) -> Self {
        Self {
            state: Arc::new(State {
                devices: Mutex::new(Vec::new()),
                streams: Mutex::new(Vec::new()),
                next_stream: AtomicUsize::new(0),
                live_streams: AtomicUsize::new(0),
                live_threads: AtomicUsize::new(0),
                opened: AtomicUsize::new(0),
                input_tone: Mutex::new(None),
                rendered: Mutex::new(Vec::new()),
                period,
            }),
        }
    }

    /// Adds (or re-adds after an unplug) a device and returns its id.
    pub fn add_device(
        &self,
        name: &str,
        direction: Direction,
        format: StreamFormat,
        is_default: bool,
    ) -> DeviceId {
        let id = DeviceId(format!("fake:{direction:?}:{name}"));
        self.add_device_with_id(id, name, direction, format, is_default)
    }

    /// Like [`add_device`](Self::add_device) with an explicit id, so two devices
    /// can share a name or a device can come back under a new id.
    pub fn add_device_with_id(
        &self,
        id: DeviceId,
        name: &str,
        direction: Direction,
        format: StreamFormat,
        is_default: bool,
    ) -> DeviceId {
        let mut devices = lock(&self.state.devices);
        devices.retain(|d| d.id != id);
        if is_default {
            for other in devices.iter_mut().filter(|d| d.direction == direction) {
                other.is_default = false;
            }
        }
        devices.push(DeviceInfo {
            id: id.clone(),
            name: name.to_owned(),
            direction,
            is_default,
            format,
        });
        id
    }

    /// Removes a device, as unplugging a headset does. Every stream open on it
    /// stops delivering audio and reports [`DeviceError::Lost`].
    pub fn unplug(&self, id: &DeviceId) {
        lock(&self.state.devices).retain(|d| &d.id != id);
        let affected: Vec<(StreamErrorCallback, DeviceError)> = lock(&self.state.streams)
            .iter()
            .filter(|(_, s)| &s.device == id)
            .map(|(_, s)| {
                s.lost.store(true, Ordering::Release);
                (
                    Arc::clone(&s.on_error),
                    DeviceError::Lost {
                        direction: s.direction,
                        name: s.name.clone(),
                    },
                )
            })
            .collect();
        for (callback, error) in affected {
            callback(error);
        }
    }

    /// Makes input streams deliver a sine of `hz` (full scale 0.5) instead of
    /// silence. Affects streams opened afterwards and ones already running.
    pub fn set_input_tone(&self, hz: Option<f32>) {
        *lock(&self.state.input_tone) = hz;
    }

    /// Streams that are open right now.
    pub fn live_streams(&self) -> usize {
        self.state.live_streams.load(Ordering::Acquire)
    }

    /// Threads the fake started that have not ended.
    pub fn live_threads(&self) -> usize {
        self.state.live_threads.load(Ordering::Acquire)
    }

    /// Streams opened since creation.
    pub fn opened_total(&self) -> usize {
        self.state.opened.load(Ordering::Acquire)
    }

    /// Everything the output streams have rendered, oldest first.
    pub fn rendered(&self) -> Vec<f32> {
        lock(&self.state.rendered).clone()
    }

    fn device_present(&self, device: &DeviceInfo) -> bool {
        lock(&self.state.devices).iter().any(|d| d.id == device.id)
    }

    fn start_stream(
        &self,
        device: &DeviceInfo,
        on_error: StreamErrorCallback,
        mut tick: impl FnMut(&State, &mut Vec<f32>, usize) + Send + 'static,
    ) -> Result<Box<dyn AudioStream>, DeviceError> {
        if !self.device_present(device) {
            return Err(DeviceError::NotFound {
                direction: device.direction,
                name: device.name.clone(),
            });
        }
        let stop = Arc::new(AtomicBool::new(false));
        let lost = Arc::new(AtomicBool::new(false));
        let id = self.state.next_stream.fetch_add(1, Ordering::AcqRel);
        lock(&self.state.streams).push((
            id,
            OpenStream {
                device: device.id.clone(),
                name: device.name.clone(),
                direction: device.direction,
                lost: Arc::clone(&lost),
                on_error,
            },
        ));
        self.state.live_streams.fetch_add(1, Ordering::AcqRel);
        self.state.opened.fetch_add(1, Ordering::AcqRel);
        self.state.live_threads.fetch_add(1, Ordering::AcqRel);

        let frames = (u64::from(device.format.sample_rate) * self.state.period.as_millis() as u64
            / 1000) as usize;
        let samples = frames * usize::from(device.format.channels);
        let state = Arc::clone(&self.state);
        let thread_stop = Arc::clone(&stop);
        let thread_lost = Arc::clone(&lost);
        let spawned = std::thread::Builder::new()
            .name("fake-audio-stream".to_owned())
            .spawn(move || {
                let _alive = Decrement(&state.live_threads);
                let mut block = vec![0.0_f32; samples];
                let mut position = 0_usize;
                while !thread_stop.load(Ordering::Acquire) {
                    if !thread_lost.load(Ordering::Acquire) {
                        tick(&state, &mut block, position);
                        position += frames;
                    }
                    std::thread::park_timeout(state.period);
                }
            });
        let handle = match spawned {
            Ok(handle) => handle,
            Err(e) => {
                lock(&self.state.streams).retain(|(sid, _)| *sid != id);
                self.state.live_streams.fetch_sub(1, Ordering::AcqRel);
                self.state.live_threads.fetch_sub(1, Ordering::AcqRel);
                return Err(DeviceError::Backend(e.to_string()));
            }
        };
        Ok(Box::new(FakeStream {
            id,
            stop,
            handle: Some(handle),
            state: Arc::clone(&self.state),
        }))
    }
}

struct Decrement<'a>(&'a AtomicUsize);

impl Drop for Decrement<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

struct FakeStream {
    id: usize,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    state: Arc<State>,
}

impl AudioStream for FakeStream {}

impl Drop for FakeStream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            handle.thread().unpark();
            let _ = handle.join();
        }
        lock(&self.state.streams).retain(|(id, _)| *id != self.id);
        self.state.live_streams.fetch_sub(1, Ordering::AcqRel);
    }
}

impl AudioBackend for FakeBackend {
    fn devices(&self, direction: Direction) -> Result<Vec<DeviceInfo>, DeviceError> {
        Ok(lock(&self.state.devices)
            .iter()
            .filter(|d| d.direction == direction)
            .cloned()
            .collect())
    }

    fn open_input(
        &self,
        device: &DeviceInfo,
        mut callback: InputCallback,
        on_error: StreamErrorCallback,
    ) -> Result<Box<dyn AudioStream>, DeviceError> {
        let rate = device.format.sample_rate as f32;
        let channels = usize::from(device.format.channels);
        self.start_stream(device, on_error, move |state, block, position| {
            let tone = *lock(&state.input_tone);
            for (i, frame) in block.chunks_exact_mut(channels).enumerate() {
                let value = tone.map_or(0.0, |hz| {
                    0.5 * (2.0 * PI * hz * (position + i) as f32 / rate).sin()
                });
                frame.fill(value);
            }
            callback(block);
        })
    }

    fn open_output(
        &self,
        device: &DeviceInfo,
        mut callback: OutputCallback,
        on_error: StreamErrorCallback,
    ) -> Result<Box<dyn AudioStream>, DeviceError> {
        self.start_stream(device, on_error, move |state, block, _| {
            callback(block);
            let mut rendered = lock(&state.rendered);
            if rendered.len() + block.len() <= RENDER_LIMIT {
                rendered.extend_from_slice(block);
            }
        })
    }
}
