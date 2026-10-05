# audio-io

Capture, playback, resampling, the microphone gate, device handling and session
lifecycle for Lumingo (roadmap tasks S2-01, S2-02, S2-03, S2-06, S2-07, S2-08).

All logic runs against fake devices in tests. The real `cpal` backend is behind the
off-by-default `cpal-backend` feature.

## Parts

| Module | What it does |
|---|---|
| `ring` | Lock-free single-producer single-consumer capture ring (`rtrb`). The device callback copies a whole block or drops it and counts it. |
| `resample` | `MonoResampler` (any rate to any rate, streaming) and `CaptureConverter` (any device rate and channel count to 16 kHz mono `f32`). `rubato` 5: FFT resampler when the rates reduce to small integers, sinc for odd rates. |
| `playback` | `playback_queue` gives a `PlaybackQueue` (producer side, converts TTS chunks to the device rate) and a `PlaybackSource` (the output callback). Stop flushes on the next buffer. Underruns are counted. |
| `gate` | `MicGate`: frames are dropped while playback is active and for 150 ms after. `PushToTalk` is a shared flag. |
| `capture` | `CapturePath`: ring, converter, framer, gate. Pure logic a worker calls in a loop. |
| `device` | `AudioBackend` seam, `DeviceRegistry` (list, select, remember, resolve), `PrefsStore` (memory and JSON file). |
| `session` | `AudioSession`: streams, capture worker, playback handle, events, `recover`, `cancel`, `stop`. |
| `fake` | Fake backend, only with the `test-support` feature or in this crate's own tests. Never in a release build. |

## Queues and what happens when they are full

| Queue | Capacity | When full |
|---|---|---|
| Capture ring | 2 s of device audio (`SessionConfig::capture_buffer`) | The callback block is dropped whole and `overflow_events` and `dropped_samples` go up. It never blocks. |
| Playback queue | 30 s of device audio (`playback_capacity`) | `enqueue` returns `PlaybackError::QueueFull` and queues nothing of that chunk. |
| Session events | 32 (`event_capacity`) | The new event is dropped and counted in `SessionStats::events_dropped`. The sender can be a device thread, so it must not wait. |

## Behaviour worth knowing

* **Gate time is counted in captured samples**, not wall-clock time. The hold-off
  starts at the first frame seen without playback and lasts at least 150 ms,
  rounded up to whole frames. If the capture worker falls far behind, frames that
  were recorded during playback but are processed after it ended are not caught;
  keep the worker current (the ring overflow counter shows when it is not).
* **Push-to-talk.** While the flag is set, frames take `Route::PushToTalk`: they
  bypass both the VAD and the playback gate, because the learner chose to speak.
  The consumer treats everything between key press and release as one utterance.
  Stopping the tutor's speech when the key goes down is the caller's decision
  (`PlaybackHandle::stop`).
* **Underrun** means the output callback ran dry while a turn was open: audio was
  queued and the producer had not called `finish_turn`. Call `finish_turn` right
  after the last chunk of a reply. A dry callback after `finish_turn` or after a
  `stop` is a normal end.
* **`stop` is a flush by position.** It records how much had been queued, and the
  callback discards up to that position at the start of its next buffer. Chunks
  that were being converted while `stop` ran are discarded too.
* **Device loss.** A lost stream raises `AudioEvent::DeviceLost`. A lost output
  also flushes the playback queue so the gate cannot stay shut. `recover` reopens
  the remembered device, or the default one if it is gone, and swaps the capture
  input or the playback queue in place. The session keeps running.
* **Lifecycle.** `stop` (and `Drop`) cancels the shared `CancelFlag`, closes both
  streams, and joins the capture worker and every thread added with
  `spawn_worker`. A backend stream must end its own threads when dropped.

## Tests

```
cargo test -p audio-io
```

`tests/lifecycle.rs` runs 100 start and stop cycles and compares thread count and
open handle count (Linux, through `/proc`) before and after. It contains a single
test because it counts for the whole process.

## Real backend: UNVERIFIED

`--features cpal-backend` builds `CpalBackend` on `cpal` 0.18.2. It has only been
compile-checked:

```
cargo check  -p audio-io --features cpal-backend --target x86_64-pc-windows-msvc
cargo clippy -p audio-io --features cpal-backend --target x86_64-pc-windows-msvc -- -D warnings
```

Both pass. On Linux the same feature needs the ALSA development files
(`libasound2-dev`), which the build container does not have, so it was not checked
there. No device has ever been opened by this code.

What the owner must run on Windows with a microphone and speakers:

1. List devices: names and default flags match the Windows sound settings, and a
   headset appears with its own rate and channel count.
2. Open the default input and output, play three TTS chunks, and hear them gapless
   and in order. Stop on command and confirm silence within 50 ms.
3. A 60-second capture reports `overflow_events` of zero.
4. Unplug a headset during capture: `DeviceLost` arrives, one `recover` call
   continues on the default device.
5. After many start and stop cycles, the process thread count and handle count in
   Task Manager or Process Explorer return to where they started.
6. Check that dropping a `cpal` stream joins its thread on WASAPI (the cpal source
   does so; this has not been observed).
7. A device whose default format is not `f32`, `i16`, `u16` or `i32` is refused with
   a typed error. Confirm that the devices in use are not affected.
