# pron-engine

Experimental pronunciation scoring for Lumingo. Covers ROADMAP S3-08 (lexicon
and phone map), S3-09 (posteriors, alignment, GOP, scores) and S3-11
(performance tier and adaptive mode, ADR-009). S3-10 (validation and
thresholds) needs the owner's recordings and is not done.

## What is in it

| Module | What it does |
|---|---|
| `arpabet`, `lexicon`, `reference` | The 39 ARPAbet symbols and their classes. A loader for the `cmudict.dict` text format with variants. Text normalisation. Words the lexicon cannot pronounce become "not checked" with a reason, never a guess. |
| `phone_map`, `vocab` | `data/phone_map.toml` maps every ARPAbet symbol to the labels of the phoneme model. Binding it to a model vocabulary fails if any symbol would be left with no label. |
| `posteriors`, `align`, `gop` | A plain `LogPosteriors` matrix (frames by labels). CTC forced alignment as one Viterbi pass over a word graph, so pronunciation variants compete over the same frames. GOP per aligned phone with the heard label. All pure functions. |
| `calibration`, `score` | Thresholds per phone class and the logistic curve as configuration. Per-phoneme, per-word and per-utterance results, focus phonemes, free-speech highlights (at most three words). |
| `perf` | The speed measurement harness, the `blocking` or `deferred` policy and the stored speed profile. |
| `model`, `ort_backend` | The `PosteriorModel` trait. The ONNX Runtime adapter is behind the `ort-backend` feature. |
| `bin/pron` | `pron score` and `pron mode` (feature `cli`). |

## Honesty rules the code enforces

- A phoneme result exists only for a phone the alignment placed on frames.
  Unchecked words and unalignable audio have no phoneme list, only a reason.
- "Not scored" is its own state: `Outcome::NotScored` with a reason (rejected
  free-speech transcript, no word in the lexicon, no audio, audio too short for
  the text). It is never a zero.
- **The thresholds and the score curve are unvalidated.** The shipped
  configuration, `Calibration::unvalidated()`, has none. With it the engine
  reports measured GOP, aligned frames and the heard label, but no 0..1 score
  and no flag. Values appear only when a calibration file supplies them, and
  the report then says `calibration_validated: false` until the file names
  where it was validated. Choosing them is S3-10. The values used inside the
  tests are marked as test values and prove only that the logic works.
- Every report carries `experimental: true`.

## Lexicon

No dictionary is bundled. `Lexicon::from_path` reads a `cmudict.dict` file you
supply. CMUdict is distributed under its own BSD-style terms by Carnegie Mellon
University; the licence register records what was checked on crates.io and why
no crate is a dependency. `tests/data/test_lexicon.dict` is a small
hand-written lexicon for the tests. It says in its first lines that it is not
CMUdict and its entries are not authoritative.

To run the loader on a real dictionary:
`LUMINGO_CMUDICT=/path/to/cmudict.dict cargo test -p pron-engine -- --ignored`.
It was run once in the build container against the data file inside the
`cmudict-fast` 0.8.0 package: it loaded without error (more than 100,000
words) and all 39 symbols were recognised.

## Phone map: candidate, unverified

The phoneme model is chosen in S1-07, and the build container cannot reach
Hugging Face, so `data/phone_map.toml` was written for the espeak-style
candidate (`facebook/wav2vec2-lv-60-espeak-cv-ft`) from the espeak English
phoneme inventory, not from that model's `vocab.json`. Its `status` says
`candidate-unverified-against-vocab`. When a model is chosen, bind the map to
its vocabulary: `PhoneMap::bind` returns an error naming the first symbol left
without a label, and the map can be edited or replaced (`pron score
--phone-map`).

## Known limits

- Alignment can place a badly said phone on the cheapest nearby frames (a
  neighbour's blank, for instance), so one wrong sound may be measured on fewer
  or different frames than it was spoken on. GOP is a mean over the frames it
  gets.
- A word that was not checked leaves its audio between the aligned words.
  Neighbouring words are marked `adjacent_to_unchecked` and their numbers
  deserve less trust. A wildcard unit for unchecked words was considered and
  not added: without a penalty it takes frames from real words, and the penalty
  would be an invented number.
- Competing labels in GOP are all non-blank labels, including any special
  tokens a vocabulary has.
- Stress, rhythm and intonation are not assessed.

## Features

| Feature | Adds | Status |
|---|---|---|
| (none) | Everything above except the adapter and the program | Tested |
| `cli` | The `pron` program: `score --posteriors FILE.json`, `mode` | Tested end to end by `tests/cli.rs` |
| `ort-backend` | `OrtPosteriorModel` (ort 2.0.0-rc.13, `load-dynamic`) and WAV input for `pron score` | **UNVERIFIED** |

### UNVERIFIED: `ort-backend`

The adapter compiles (`cargo check -p pron-engine --features ort-backend`, also
for `x86_64-pc-windows-msvc`) and passes clippy. It has never run: the
container has no ONNX Runtime library and no phoneme model. API calls were read
from the source of `ort` 2.0.0-rc.13 in the cargo registry. Assumptions only a
real model can confirm: one `[1, samples]` float32 input normalised to zero mean
and unit variance, a first output of `[1, frames, labels]`, and labels equal to
the vocabulary size. `ort` with `load-dynamic` panics if the library is missing,
so the adapter loads it first and reports an error instead.

What the owner runs on Windows, with a model and an ONNX Runtime DLL:

```text
cargo run -p pron-engine --features ort-backend --bin pron -- score ^
  --text "I think the ship is full" --lexicon cmudict.dict ^
  --vocab vocab.json --model phoneme.onnx --runtime onnxruntime.dll speech.wav
```

The WAV must be 16 kHz mono; the program does not resample. Check that the
shape error, if any, names the real output shape, then check the phone map as
described above.

## Timing mode (S3-11)

`perf::measure` times caller-supplied workloads (STT, the phoneme model,
alignment) on a fixture: one warm-up run, then the median of N runs, STT and
model alone and together. `perf::decide` predicts the wait the analysis adds
before the LLM request for an utterance of a configured length: the part of the
model run that outlasts STT, plus alignment. It returns `blocking` when that is
within `max_added_wait_ms`, otherwise `deferred`; the setting overrides it, and
with nothing measured `automatic` means `deferred`. `max_added_wait_ms` has no
default. It comes from the latency budget (`headroom_ms` does the subtraction).
Wiring a real STT workload and the bundled audio fixture needs S3-06 and the
chosen models.
