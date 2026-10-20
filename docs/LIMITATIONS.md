# Limitations

This is the list of what nobody has measured yet, written so a reader can
decide what to trust. It is not an apology and it is not a plan with dates in
it. Where a number appears below, it is either a target the project set for
itself or a count of files in the tree — never a measurement of how the
program performs.

Read [INSTALL.md](INSTALL.md) for what the build needs, [MODELS.md](MODELS.md)
for what it ships and does not ship, and [PRIVACY.md](PRIVACY.md) for what
leaves the machine. The owner's release steps are in
[OWNER_RELEASE.md](OWNER_RELEASE.md); the notes that travel with a build are
in [RELEASE_NOTES.md](RELEASE_NOTES.md).

## What is not measured

- **The engine choices are unmeasured.** The speech-to-text (STT),
  text-to-speech (TTS), voice-activity-detection (VAD) and phoneme model
  candidates have never been run against a model file, on any machine, by
  anyone. The `sherpa` adapters have never been linked, never loaded a model
  and never produced a transcript or a sound. The ONNX Runtime path compiles
  and has never run against a model either. Which engines the program ships
  with is still an open decision, not a settled one.
- **The end-to-end latency target is unmeasured.** PRD NFR-P1 asks for a p95 of
  at or under `3000 ms` from the last speech sample to the first tutor audio
  sample. No run has measured it. **No latency figure is claimed anywhere in
  this document.** The `3000 ms` is a target written down in advance; nothing
  here says the program reaches it.
- **Gate G1 is not met.** That gate asks for chosen STT, TTS and VAD settings
  and a phoneme model, each with result files, for the `3000 ms` target to be
  confirmed as reachable or revised in a decision record, and for the owner's
  decision on one open question. None of those conditions is satisfied.
  **No gate in the roadmap has been approved**, and every gate's date field
  is still empty.
- **The benchmark recordings the plan called for were skipped.** The
  benchmark plan's fixture sets F2, F3, F5 and F6 are recordings the owner has
  to make: read scripts, room noise, deliberate sound substitutions. Only F1,
  the generated sweep set, is in the repository. The suites that need the
  owner's voice — the STT, TTS, VAD and phoneme suites — therefore never ran,
  and `benchmarks/results/` holds no engine-suite result file. The outcome is
  not a bad number. It is no number.

## The engines

The speech engines sit behind Cargo features that are **off by default**:

- `sherpa` in `crates/speech` — the real VAD, STT and TTS adapters.
- `cpal-backend` in `crates/audio-io` — the real microphone and speaker
  backend.
- `ort-backend` in `crates/pron-engine` — the ONNX Runtime adapter that reads
  a WAV file for phoneme scoring.

`apps/server` re-exports `sherpa` and `cpal-backend`, and the release
pipeline builds the server with neither. So the default build has **no speech
engine** and **no microphone**. The speaking steps — the roleplay, the
read-aloud drill, the pronunciation check — cannot be run from this build at
all. They are not slow; they are absent.

The models are in neither the repository nor the release. Every entry in
`models/manifest.toml` carries `version = "candidate"` and an empty `files`
list, and the model manager refuses such an entry before it makes any network
request. **No model can be downloaded yet**, and nothing has been downloaded
from `github.com` or `huggingface.co`, the two hosts the manifest names as
the sources for its candidates. The download path checks the URL and not the
host, so a redirect could reach another HTTPS host. In the release build the
program looks for `models/manifest.toml` next to the executable, and the zip
holds no `models/` folder, so the manifest is not found there. The server's
own reason is that the model manifest cannot be read, and the settings screen
lists no rows: it says the model list could not be loaded. In a source
checkout the manifest is found in the working directory instead, the settings
screen lists the ten manifest rows, and each row carries the server's own
reason that it cannot be fetched.

## The release build

The release truth is a Windows x64 zip with five members — `.env.example`,
`LICENSE`, `NOTICE`, `README.md` and `lumingo.exe` — and a `checksums.txt`
sibling. The program serves its interface on `http://127.0.0.1:8765`.

- **The Windows half of the pipeline is reviewable and correct but
  unproven.** The pipeline runs on `windows-latest` in CI and has been read
  carefully. No `windows-latest` run exists: CI was never executed, and
  **nobody has run the Windows binary**. The only local build was a `Linux`
  stand-in — the same packaging code, built on `Linux`, and named
  `linux-standin` precisely so that it cannot be mistaken for the release
  artifact. It says the packaging and the interface embedding work. It says
  nothing about the Windows binary, the Windows runner, or the size of the
  Windows zip against the size cap.
- **The zip carries no curriculum.** The curriculum is not one of the five
  members, and the shipped program starts with an empty unit list. The
  clean-account walk of PRD 13.8 — a fresh Windows 11 account unpacking the
  zip and completing unit A1-01 from the README alone — is therefore **not
  reachable from this artifact**. No claim is made that it is.
- **The program is unsigned.** Windows SmartScreen shows a warning on the
  first run, because Windows cannot check who published it. The README in the
  zip explains the warning, and the sums in `checksums.txt` let the reader
  check the five files themselves.
- **The default build has no microphone and no speech engines.** The speaking
  steps in the owner's release walk are recorded as untested, never as
  passed.

## The estimates

Level estimates are computed on the learner's own machine, from their own
recorded work, and are always labelled as estimates. Each one carries a
confidence figure and an evidence drill-down: the attempts behind it, with
the response, the quotes and the scorer's reason, so the number can be
checked rather than believed.

- A free-mode session never counts toward an estimate.
- A generated item never counts toward an estimate.

Pronunciation feedback is experimental, and every screen that shows it says
so. The rule-based grammar checker that backs part of that feedback has a
gap that was measured, not assumed: it does not catch subject-verb
agreement, which is the most common error of the target learners, so what it
reports is a weak alarm rather than a verdict. The measurements and the
sentences they were taken on are in `docs/GRAMMAR_CHECK.md`.

## What would close the gap

Each gap below has an owner and a thing that would close it. No dates: the
order is the owner's to set.

| Gap | Owner | What closes it |
|---|---|---|
| No engine has been measured | The owner | The benchmark suites run on the owner's machine, with the F2, F3, F5 and F6 recordings made and result files written |
| The `3000 ms` target is unmeasured | The owner | The same run on the 8 GB profile, producing the end-to-end result files |
| Gate G1 is not met, and no gate is approved | The owner | The owner's approval, with the date written next to the gate |
| The Windows half of the pipeline is unproven | The owner | A real `windows-latest` run of the release pipeline, on the tagged commit |
| The zip cannot complete a unit | The owner | A curriculum in the artifact, which changes the member list and the size gate |
| No model can be downloaded | The owner | Real manifest entries: a file path, a SHA-256 of a file the owner downloaded and checked by hand, a size, and a licence read at the source |
| The default build has no microphone and no speech engine | The owner | A device day on a real 8 GB machine with the audio features enabled, and the network capture taken during a full session |
| The program is unsigned | The owner | A signing step in the pipeline, and the decision of whether a release should carry one at all |
