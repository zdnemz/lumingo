# Lumingo 1.0.0

Lumingo 1.0.0 is the Windows x64 build of the English tutor. This note is the
text of the release, so it says what the artifact contains and, in as much
detail, what has and has not been checked.

## What is in this release

- **Version:** `1.0.0`.
- **Asset:** `lumingo-1.0.0-windows-x64.zip` — a flat archive with exactly five
  members: `.env.example`, `LICENSE`, `NOTICE`, `README.md` and `lumingo.exe`.
- **Checksums:** `checksums.txt`, a sibling of the zip and not inside it, in
  `sha256sum` format: one line per member, covering exactly those five files.

Lumingo is an English tutor that runs on your own computer: listening,
speaking, reading and writing as four-skill units on a pixel-art quest map,
plus a writing workshop, graded reading, free chat, and a voice conversation
with a tutor.

By default the program listens on the loopback address
`http://127.0.0.1:8765` and serves its pages from there.

## Known limits

- **The program is unsigned.** Windows cannot check who published it, so
  SmartScreen warns on the first run. Choose "More info", then "Run anyway".
- **The zip carries no curriculum.** No unit is in the archive, so the unit
  list starts empty and no unit can be completed from this artifact.
- **The default build has no microphone and no speech engines.** The speaking
  steps cannot be run at all. The parts that use text work.
- **The `3000 ms` end-to-end latency target is unmeasured.** Nothing here has
  been measured on the target machine, no latency figure is claimed, and
  `Gate G1` is not met.
- **The Windows half of the pipeline is unproven.** There is no
  `windows-latest` CI run of the release workflow, nobody has run the Windows
  binary, and the only local build was a **Linux** stand-in, named
  `linux-standin` so it cannot be mistaken for the release.

## What has been checked

- **The workspace test suite** (`cargo test --workspace`): the Rust crates
  the release is built from, including the unit tests behind the pipeline's
  own pieces — the server's embedded interface, the curriculum loader and the
  model manifest.
- **The web test suite** (`pnpm --dir apps/web test`): the interface's
  components and states.
- **The static-export guard** (`scripts/check-export-only.mjs`): a scan of
  `apps/web` for the shapes a static export cannot support. It was proved to
  fail — `route.ts` and `"use server"` probes were planted, each was caught
  and named on stderr, and the tree was clean afterwards.
- **The packaging script and its checksums**
  (`scripts/package-release.mjs`): the flat archive, the five members, and
  `checksums.txt` in `sha256sum` format, with digests equal to `sha256sum` of
  the unpacked members.
- **The pipeline's pieces agree with each other:** the release workflow's
  flags match the packaging script's own `--help`, the guard's step runs
  before the web build, and the README inside the zip explains the
  SmartScreen warning.
- **The local Linux stand-in build:** a real release build of the server with
  the export embedded. The served page carried the built static assets and
  not the "has not been built" page, and the packaging turned the binary into
  a `linux-standin` zip. That is an indication that the packaging and the
  embedding work. It says nothing about the Windows binary, the Windows
  runner, or the size of the Windows zip.

## What has not been checked

- **The Windows binary.** Nobody has run `lumingo.exe` on Windows.
- **A real `windows-latest` CI run** of the release workflow. The pipeline is
  reviewable and unproven.
- **The clean-account walk** on a fresh Windows 11 machine with no Node and no
  Rust: unpack, check the sums, run the program, set up the provider.
- **The live network capture.** No packet capture of a full session exists, so
  what leaves the machine has not been shown on the network. See
  [PRIVACY.md](PRIVACY.md).
- **A real 8 GB device day.** The floor profile — 8 GB of RAM, 4 threads, no
  GPU — has not been measured on a real 8 GB machine.
- **The speech engines and the models.** The default build has no microphone
  and no speech engines, and no entry in the model manifest can be downloaded
  yet, so nothing speech-related has been run. See [MODELS.md](MODELS.md).

[OWNER_RELEASE.md](OWNER_RELEASE.md) holds the owner's four steps — push, tag
`v1.0.0`, the draft release, the clean-account walk — each with the exact
command and what to write down.
[LIMITATIONS.md](LIMITATIONS.md) has the full list of known limits.

## Install

1. Unpack `lumingo-1.0.0-windows-x64.zip` into a folder. Do not run the
   program from inside the zip.
2. Copy `.env.example` to `.env` in that folder. The name must be exactly
   `.env`.
3. Fill in the four settings in `.env`: `TUTOR_LLM_PROTOCOL`,
   `TUTOR_LLM_BASE_URL`, `TUTOR_LLM_MODEL` and `TUTOR_LLM_API_KEY`. The values
   come from the AI service you signed up for; none of them is included here.
4. Run `lumingo.exe`. A window opens, it prints the address it is using, and
   your browser opens there. The default address is `http://127.0.0.1:8765`.
5. If the browser does not open by itself, paste the printed address into it.
   If port `8765` is already taken, the program takes the next free port and
   prints that address instead — use the one it prints.

Check the sums before you run anything: compare the five unpacked files with
their lines in `checksums.txt` (`Get-FileHash -Algorithm SHA256` in
PowerShell).

The full guide, including the Windows SmartScreen warning, is in
[INSTALL.md](INSTALL.md).
