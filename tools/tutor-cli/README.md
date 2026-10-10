# tutor-cli

The Lumingo voice loop on the command line (ROADMAP S3-06 and S3-12). It listens,
transcribes, thinks and speaks with the T1 prompt and a fixed scenario from a unit
file, before the UI exists. The loop itself is `app_core::voice`; this crate is the
terminal around it: arguments, provider choice, the audio and speech side, scripted
runs and the result file.

**Nothing here has been run with a microphone, a speaker or a speech model.** The
build container has none of them. Real provider runs have happened: the owner's
`tutor-cli probe` (2026-10-10), then `unit practice` and `chat --analysis` against
the gateway the same day (see "What is verified"). Read "What is verified" and
"UNVERIFIED" before you trust a sentence of this file.

## Build

| Build | What it contains |
|---|---|
| default | The loop with typed input (`--text`), scripted text runs and the result file. No audio device, no speech engine, no fakes. `--backend cpal` and voice input fail with exit code 4 and name the feature to build with. |
| `--features cpal-backend` | Microphone and speakers through cpal (`audio-io`). UNVERIFIED. |
| `--features sherpa` | Real VAD, recogniser and synthesiser through sherpa-onnx (`speech`). UNVERIFIED. |
| `--features test-support` | `--backend fake`: fake devices and fake engines. For tests only; a run says so in its result file. A default or release build does not contain them and refuses `--backend fake`. |

The Windows commands below use PowerShell and forward slashes.

```powershell
cargo build --release -p tutor-cli                                  # typed input only
cargo build --release -p tutor-cli --features cpal-backend,sherpa   # the real thing
```

## The provider

The same sources as the server. Set the four variables in the environment or in a
`.env` file in the working directory (never commit it):

```
TUTOR_LLM_PROTOCOL=openai_chat        # or anthropic_messages
TUTOR_LLM_BASE_URL=https://...        # https, or http on loopback only
TUTOR_LLM_MODEL=...
TUTOR_LLM_API_KEY=...
```

They make the read-only profile `env`. Profiles saved by the settings screen live in
`providers.toml` in the data directory (`--data-dir`, `%LOCALAPPDATA%/Lumingo` by
default). `--provider <name>` picks one; with a single profile that one is used. The
program prints the host and the model and nothing else about a profile. Every request
goes through the `llm-client` host allowlist: the only host contacted is the provider's.

## Usage

```
tutor-cli chat [OPTIONS]
```

| Option | Meaning |
|---|---|
| `--scenario <file>` | The unit whose roleplay is the scenario. Default `curriculum/examples/a1-u01.example.json` (the roleplay `a11-roleplay-classmate`). |
| `--activity <id>` | Another roleplay of the unit. |
| `--mode fluency\|accuracy` | Feedback mode. The roleplay's own mode when not given. |
| `--provider <name>` | Provider profile. |
| `--backend cpal\|fake` | Audio backend for voice input. Default `cpal`. |
| `--text` | Typed input: no microphone, no VAD, no recogniser. Replies are printed. |
| `--speak` | Speak the replies although the input is typed (needs TTS and an output device). Voice input speaks them by default. |
| `--no-speak` | Do not speak the replies. |
| `--script <file>` | A script of turns (below). The run needs no person. |
| `--turns <N>` | With a script: the number of scripted turns (a short script repeats). Without one: stop after N learner turns. |
| `--out <file>` | Result file of a scripted run. Default `tutor-cli-result.jsonl`. |
| `--learner-first` | The learner speaks first. By default the tutor opens the conversation. |
| `--provider-timeout-ms` | The whole time the provider gets to start a reply for a turn, both attempts together. Default 16000. |
| `--end-silence-ms` | Silence that ends an utterance, 400 to 900. Default 600. |
| `--analysis` | Store the session in the database and run the background analysis (T2). Without it nothing is stored. |
| `--engines <file>` | The sherpa models (below). |
| `--show-states` | Print every phase of the session. |

In a conversation, a line you type is a turn too. `/stop` silences the tutor, `/pause`
and `/resume` work as named, `/quit` ends. Ctrl-C stops everything cleanly and exits
with 130. Every turn prints its latency parts:

```
latency  endpointing 608 ms | stt 700 ms | llm 900 ms | tts 300 ms | output 50 ms | sum 2558 ms
```

(that line is an example of the format, not a measurement.) A part a turn did not go
through, such as the endpointing wait of a typed turn, is shown as `-`.

### Exit codes

| Code | Meaning |
|---|---|
| 0 | Finished. |
| 1 | Anything else: a bad file or option, no provider configured, an internal error. |
| 2 | A usage error (clap). |
| 3 | The provider could not be reached or kept failing (`ProviderUnavailable`). The session was stopped cleanly, and a scripted run still wrote its result file. |
| 4 | A speech engine or an audio device is not available or did not load. |
| 130 | Ctrl-C. |

### The latency parts

Measured inside the loop with one injected clock, from the last sample the VAD
classified as speech to the first tutor sample the output callback consumed
(`context_pack.md` section 4). The five parts add up to the whole by construction.
"Output start" is found by polling the playback counters every 2 ms, so it carries that
resolution, and it includes the conversion of the audio to the device rate.

## `probe`: the capability probe on the command line

```
tutor-cli probe [OPTIONS]
```

It runs the connection test (`docs/PROMPT_CONTRACTS.md` section 4) against the
provider and prints what each step found: auth, streaming, the structured-output
ladder level and the time to the first token. (The per-contract results live in
the `Capabilities` object the probe returns; the command line does not print
them.) The protocol and the model name are printed; the key never is.

```
provider 127.0.0.1 (openai_chat, model some-model)
probe: auth true, stream true, structured level Some(2), first token Some(640) ms
```

The owner's run from the merged tree, 2026-10-10, against the gateway (the model
name is left out; it is configuration, not part of the record):

```
provider localhost (anthropic_messages, model <configured>)
probe: auth true, stream true, structured level Some(2), first token Some(1285) ms
```

Every command that makes structured calls (`chat --analysis`, `unit run` with a
provider, `unit score-pending`, `unit practice`) runs the probe first for the same
reason, so the calls start at the ladder level the provider really supports. Without
the probe a gateway that silently ignores the native schema (found live on 2026-10-07)
would fail every structured call at level 1. `TUTOR_LLM_FORCE_LEVEL=1..4` forces a
level after the probe, to compare what a provider does at each one.

Exit codes: 0 when a structured level worked, 1 when none did, 3 when the provider
could not be reached or rejected the key.

## `unit`: play a unit to its checkpoint

```
tutor-cli unit run <unit.json> --script <responses.json> [--db <file>] [--out <file>] [--offline]
tutor-cli unit run <unit.json> --interactive
tutor-cli unit score-pending --db <file>
tutor-cli unit practice <unit.json> --script <responses.json> [--count <n>] [--db <file>] [--offline]
```

`unit run` plays every activity of the unit in order with `tutor_engine::UnitPlayer`,
prints the activity and then its task score (0 to 1, never a level), stores one
attempt row per scored dimension with its evidence, and decides the unit checkpoint
from the stored rows. The database is a new file in the temporary folder unless
`--db` is given; its path is printed. The example unit and a matching script:

```
cargo run -p tutor-cli -- unit run curriculum/examples/a1-u01.example.json \
  --script tools/tutor-cli/scripts/a1-u01-responses.json --offline
```

* **Provider.** The same sources as `chat`. With none (or `--offline`) the objective
  activities are scored normally and the productive ones (`guided_speaking`,
  `guided_writing`, `mediation`, a scored `roleplay`) are stored as `pending_llm` and
  queued; the checkpoint is then provisional and the unit stays `in_progress`. When a
  provider is reachable, `unit score-pending --db <file>` scores the queue and exits
  with 3 if the provider cannot be reached.
* **Rubrics** come from `--rubrics` or `catalogs/rubrics` next to the unit's folder.
  A rubric the unit names and the folder lacks leaves that response stored as unscored,
  with the reason.
* **Script format.** One JSON object `{ "responses": { "<activity id>": { ... } } }`.
  The fields per activity type are the table at the top of `src/unit_script.rs`. A
  match is given by phrase and meaning, a listening minimal pair by the word heard,
  option indexes count from 0, `plays` is how many times audio is played first. An
  activity with no entry is skipped and listed; an entry the activity cannot take is
  refused by name and the run goes on.
* **`--interactive`** reads typed answers (numbers count from 1, `-` is blank, `/skip`
  skips, `/end` ends a roleplay). Audio items are printed as text, because this tool has
  no speaker, unless `--hide-audio-text` is given.
* **Pronunciation drills** need a recording (`wav:` or `clips`) and a pronunciation
  engine. This tool links none, so a drill is stored as not scored, with the reason.
* **`--out <file>`** writes the scores, statuses and checkpoint as JSON.

Exit codes: 0 when the checkpoint was decided, 1 when it was not (an activity of it was
skipped or refused) or on any other failure, 2 for a usage error, 3 when
`score-pending` finds no reachable provider, 130 on Ctrl-C.

### `unit practice`: extra practice (S4-10)

```
tutor-cli unit practice <unit.json> --script <responses.json> [--count <n>]
tutor-cli unit practice <unit.json> --interactive [--offline]
```

`unit practice` is "more practice" (PRD FR-L4): the model writes extra items inside
the unit's `generation_policy` (T4, `contracts/practice_items.schema.json`), each is
converted to the typed activity, checked by the same validators as authored items and
dropped when it fails, fewer than half valid means one regeneration, and when that
fails too — or there is no provider — the run replays authored items instead: the
wrong answers of the newest lesson or checkpoint run first, then the other items of an
allowed type. The run stores a `drill` session, plays the items through the same
scoring runtime a unit answer uses, and writes one attempt row per item with origin
`generated` (or `authored` for a replay) and `counts_toward_estimate = false`. The
generated items themselves are stored with the session in `generated_content` and are
deleted with it. **Nothing here can move a level estimate**, and the result file names
no level.

* **`--count <n>`** asks for up to `n` items (default 3). The unit's
  `max_items_per_session` caps the session, and items generated in this session count
  against it.
* **Script keys.** A generated item's id is scoped to its session (`gen-s<session>-1`),
  which a script written in advance cannot know, so a script answers generated items by
  position: `gen-1`, `gen-2`, ... in the order the set lists them. A replayed authored
  item keeps its authored id. An item with no entry is skipped; an entry the item cannot
  take is refused by name and the run goes on.
* **`--word-list`** (one `word,LEVEL` per line) turns on the vocabulary check against the
  unit's `max_level`. Without it the check is skipped and the run says so.
* **`--out <file>`** writes the items, their origins, their scores and why the set is
  what it is (`source`, `fallback`, `dropped`, `regenerated`) as JSON.
* Exit codes are the same as `unit run`, except that there is no checkpoint: 0 when the
  run finished, 1 on a failure, 130 on Ctrl-C.

## Scripts

One turn per line. Text is typed for the learner. `wav: <path>` is a recorded utterance
(16, 24 or 32 bit PCM or 32 bit float WAV, any rate and channel count, relative to the
script file) fed through the VAD and the recogniser at the speed it was spoken, followed
by enough silence to end the utterance. Lines starting with `#` and blank lines are
ignored. Recorded lines need the VAD and the recogniser but not a microphone.

`scripts/a1-u01-50-turns.txt` holds 50 learner lines for the example unit. Typed, they
measure the model and, with `--speak`, synthesis and output. To measure the whole chain,
read them aloud, save the clips under `benchmarks/fixtures/private/` (not in git) and
write a second script of `wav:` lines.

### The result file

JSON lines. A `run` line (input kind, backend, scenario, provider host, model and
protocol, feedback mode, the recogniser and synthesiser that loaded with their model
checksums, and a `warning` when the engines were fakes), one `turn` line per turn
(`input`, `outcome`, the five parts in ms, `sum_ms`, `complete`), then one `summary` line
with p50 and p95 per part and for the sum, and the counters of what was dropped. The sum
is taken over complete turns only, so a typed turn cannot pull the end-to-end figure
down. A file holds no learner text, no prompt, no reply and no key.

## Engines file (`--engines`, `sherpa` feature)

```toml
[vad]
model = "models/files/silero_vad.onnx"

[stt]
family = "whisper"            # or "moonshine-v2", "nemo-transducer"
encoder = "models/files/..."
decoder = "models/files/..."
tokens = "models/files/..."
language = "en"
threads = 4

[tts]                         # only when the tutor speaks
family = "kokoro"             # or "kitten", "supertonic"
model = "models/files/..."
voices = "models/files/..."
tokens = "models/files/..."
data_dir = "models/files/..."
```

The field names of each family are those of `speech::SherpaSttModel` and
`speech::SherpaTtsModel`. Paths are checked by the `speech` crate before the native
library sees them. No model is shipped here and no model is downloaded by this program.

## Owner commands

These are the commands to run on Windows with a microphone, speakers, the models and a
key. Each needs the build with `cpal-backend,sherpa` and `--engines models/engines.toml`.
Put the result files under `benchmarks/results/`.

**The 10-turn conversation on DEV (S3-06 verify):**

```powershell
cargo run --release -p tutor-cli --features cpal-backend,sherpa -- chat `
  --engines models/engines.toml --turns 10
```

The tutor opens, you speak ten times, the program stops and prints p50 and p95. Note
by ear whether the conversation works, whether the tutor stops when you type `/stop` and
whether it hears itself (it must not).

**The 50-turn scripted run, on each profile.** Boot the profile (DEV is the laptop as it
is; FLOOR is the 8 GB, 4 processor simulation, which only the owner can set up), then:

```powershell
# the model, synthesis and output with typed turns
cargo run --release -p tutor-cli --features cpal-backend,sherpa -- chat --text --speak `
  --engines models/engines.toml `
  --script tools/tutor-cli/scripts/a1-u01-50-turns.txt --turns 50 `
  --out benchmarks/results/s3-06-dev-typed.jsonl

# the whole chain with recorded utterances (script of wav: lines)
cargo run --release -p tutor-cli --features cpal-backend,sherpa -- chat `
  --engines models/engines.toml `
  --script benchmarks/fixtures/private/f8/script.txt --turns 50 `
  --out benchmarks/results/s3-06-dev-voice.jsonl
```

Repeat with `-floor` in the file names on FLOOR. Add `--provider <name>` to run the same
script against another provider profile; the result file records the host and model. G3 asks
for the end-to-end p95 of the voice run against the target; the typed run has no complete
turns and no end-to-end figure.

**Offline (S3-12 verify), by hand:** disable the network adapter, then

```powershell
cargo run --release -p tutor-cli -- chat --text --turns 1
```

It must print a message that the provider could not be reached, within the
`--provider-timeout-ms` budget (16 s by default), stop cleanly and exit with code 3:
`echo $LASTEXITCODE`.

**Extra practice with a real provider (S4-10 verify), by hand.** The probe must pass
first (the gateway quirk); the command probes on its own:

```powershell
cargo run --release -p tutor-cli -- unit practice curriculum/examples/a1-u01.example.json `
  --interactive --count 3 --out benchmarks/results/s4-10-practice.json
```

Answer the three items it prints. The lines to look at: `set: generated, 3 item(s)`,
each item's score, `every answer here is practice`, and the result file's `source`
(`generated`) — plus `dropped` and `regenerated` when the model's first answer had
problems. Then the same command with `--offline`: the set must be `authored` with
`fallback: provider_unavailable`, and the wrong answers of the newest lesson run (if
any) must come first. Nothing may change a level estimate in either run.

## What is verified

By tests that run here without hardware or network (`cargo test -p app-core -p tutor-cli`):

* the loop's turn order, sentence-by-sentence hand-over to TTS, first sentence audible
  before the model stream ends, cancellation of the model stream, the TTS turn and
  playback by stop and by speaking while the tutor thinks, the gate dropping microphone
  frames while the tutor speaks, retry once then `ProviderUnavailable`, the five latency
  parts from an injected clock, the background analysis not delaying the reply (all with
  fakes, in `crates/app-core/tests/`);
* `ProviderUnavailable` within the timeout for a closed port, an address that routes
  nowhere and a listener that never answers, and the CLI exit code 3 with a result file;
* no request to any host but the configured provider, and a redirect not followed;
* script parsing, WAV reading, result-file shape, exit codes, and a scripted voice run
  through the CLI with the fake engines;
* `unit run`: a full scripted completion of the example unit with a fake provider
  (test code only), including its reading set, listening set (with a replay limit)
  and writing tasks; the typed-input path; the offline run through the real binary
  with its pending rows, result file and `score-pending` exit code (`tests/unit.rs`);
* `unit practice` (S4-10): three generated items stored as a `drill` session with the
  raw items in `generated_content`, converted, marked `generated`, scored through the
  same runtime and stored with `counts_toward_estimate = false`; the offline fallback
  replaying the wrong answer of the newest lesson run first, keeping its `authored`
  origin and still not counting; the result file naming the source and no level; and
  the real binary offline (`tests/practice.rs`); the practice service's own fallback
  reasons, including the `NoProvider` client's answer, are covered in
  `crates/tutor-engine/tests/practice.rs`;
* the sherpa feature compiles (`SHERPA_ONNX_LIB_DIR` pointing at an empty folder, so
  nothing is linked) and the cpal device code compiles for `x86_64-pc-windows-msvc`.

By live runs the owner did from the merged tree on 2026-10-10:

* `tutor-cli probe` against the gateway (`anthropic_messages`):
  `auth true, stream true, structured level Some(2), first token Some(1285) ms`,
  exit 0 — the expected level on this gateway, where level 1's native schema is
  silently ignored and the canary walks the ladder to 2.
* `unit practice` (S4-10) against the gateway (`cx/gpt-6-luna`): probe →
  `structured level Some(2)`, first token 951 ms; `set: generated, 2 item(s),
  1 dropped` — the model's third item failed the validators and was dropped, and
  the two that passed were converted, marked `generated`, scored 1.00 on the
  owner's answers and stored with `counts_toward_estimate = false`. The result
  file is `benchmarks/results/s4-10-practice.json` (`source: generated`,
  `fallback: null`, `dropped: 1`).
* `chat --text --analysis --turns 10` against the gateway: probe → level 2,
  21 turns stored with **0 turns without an analysis**, first-sentence latency
  p50 1384 ms / p95 5059 ms over 11 tutor turns. This is the merged chat's
  T1+T2 path validated live (the 2026-10-08 run exercised the pre-merge
  implementation).

## UNVERIFIED

* **`unit run` with a real provider.** Only a fake provider answered rubric and tutor calls.
* **A two-draft workshop round** on the merged code (the owner's 2026-10-08 run
  validated the pre-merge implementation; there is no CLI surface for it yet).
* **Pronunciation drills with a real phoneme model** and any recording of a learner.

Everything below needs hardware, a model or a provider that this container does not
have. None of it has run.

* **Microphone and speakers** (`cpal-backend`): no device has ever been opened. The
  microphone gate, the 150 ms hold and the flush on stop are tested against a fake
  backend only.
* **Speech models** (`sherpa`): no model was loaded, no transcript or sound produced.
  The engines file is parsed by code that has only been compiled.
* **A real provider beyond `probe`:** the probe's five steps have run live; no
  tutor turn, analysis or rubric call has. Both protocols are otherwise tested
  against recorded fixtures in `llm-client`.
* **Any latency figure.** The tests use an injected clock and fakes; they show that the
  parts are measured and add up, not how fast anything is. The probe's first-token
  number (1285 ms) is one observation on this gateway, not a benchmark.
* **A real conversation.** Whether it works, whether the endpointing cuts learner speech
  short and whether the tutor hears itself is for the owner to judge.
* **S2-09 soak and stress** (zero dropped frames, zero underruns over 30 minutes) is not
  done and is bound to hardware.
* **The Windows build of `tutor-cli` itself.** It could not be checked for the Windows
  target in the container because `storage` builds SQLite from C source there; only the
  cpal device code was checked.
* **A black-hole address that really hangs.** In the container such an address is refused
  at once, so only the refused path ran for it. The hanging path ran against a listener that
  accepts and never answers.
