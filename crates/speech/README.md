# speech

Engine traits (`Vad`, `SttEngine`, `TtsEngine`), `CancelFlag`, `EngineInfo`, and
the pieces that sit between the audio path and an engine.

| Module | What it does | Verified |
|---|---|---|
| `endpoint` | `Endpointer` (pure logic over frame probabilities) and `UtteranceSegmenter` (adds pre-roll and the audio). 300 ms pre-roll, 200 ms minimum speech, 400 to 900 ms end silence (default 600), forced end at 30 s. | Table-driven unit tests |
| `stt_worker` | `SttWorker`: engine on its own thread, bounded input queue (default 4, a full queue hands the job back and is counted), warm-up after load, one terminal event per utterance, per-job and shutdown cancellation. | Unit tests with a fake engine |
| `tts_worker` | `TtsWorker`: engine on its own thread over sentence strings, bounded input queue (default 8) and bounded audio output (default 8, the worker waits rather than drops), `TtsTurn` numbers sentences and cancels a whole reply. The sentence chunker lives in `crates/tutor-engine/src/chunker.rs` and feeds this worker. | Unit tests with a fake engine |
| `sherpa_config` | Which files each sherpa-onnx model family needs, and the checks that run before the native library loads them. | Unit tests |
| `sherpa` (feature `sherpa`) | `SherpaVad`, `SherpaStt`, `SherpaTts` over sherpa-onnx 1.13.8. | UNVERIFIED |

## The `sherpa` feature is UNVERIFIED

It is off by default. The build script of `sherpa-onnx-sys` downloads the
native library from GitHub releases, which the build container cannot reach.
The adapters were compile-checked with `SHERPA_ONNX_LIB_DIR` pointing at an
empty folder, which makes `cargo check` and `cargo clippy` pass without
linking. They have never been linked, loaded a model or produced a transcript or
a sound.

Known limits, read from the 1.13.8 source:

- The wrapper's VAD reports `detected()`, not a probability per frame.
  `SherpaVad` returns 0.0 or 1.0, so the endpointer's hysteresis has nothing to
  work on and sherpa's own minimum durations add to ours. A real probability
  needs Silero through `ort`. The owner decides after the F2 run.
- The wrapper panics on a NUL byte in a path or in text. Configurations are
  validated first and NULs are removed from text.
- An offline decode is one native call that cannot be interrupted. Cancellation
  is checked before and after it.
- The static libraries sherpa-onnx links include espeak-ng (GPL-3.0). See
  `docs/LICENSE_REGISTER.md` section 6.

On Windows with the model files, the owner runs
`cargo test -p speech --features sherpa` (there are no tests for the adapters
yet: they need real models) and a small program that loads each adapter, feeds a
WAV file and prints the transcript and the first-audio time.
