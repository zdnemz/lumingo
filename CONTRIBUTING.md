# Contributing

Lumingo is an English tutor that runs on your own computer. This guide is for a
developer who has never seen the project: where the pieces live, what the gate
is, and the rules for committing.

## What this repository is

Lumingo trains listening, speaking, reading and writing for a learner from A1 to
C2. One Rust program owns the microphone, the speakers, the speech models and
the data, and it serves the interface on the loopback address. The learner
brings their own AI provider, and text is the only thing that leaves the
computer.

| Path | What it is |
|---|---|
| `crates/app-core` | The domain core: lifecycle, settings, typed commands and events, providers, progress and game state |
| `crates/tutor-engine` | The lesson and workshop logic: sessions, turns, prompts and the free modes |
| `crates/llm-client` | The provider adapter: two wire protocols, the capability probe, and the host allowlist every request is checked against |
| `crates/model-manager` | The speech model manifest, the resumable download and the SHA-256 check of each installed model |
| `crates/storage` | The SQLite layer: schema, migrations and repositories |
| `crates/game` | The cosmetic game layer: ranks, sparks and unlockables |
| `apps/server` | The Rust server: HTTP API, WebSocket events and the embedded interface |
| `apps/web` | The interface, built as a static export in English and Indonesian |
| `tools/tutor-cli` | The command-line tutor, which plays the same engine as the server |
| `tools/content-cli` | The content tooling: validates the curriculum units and catalogs |
| `tools/bench` | The benchmark harness for the engines |
| `curriculum/`, `contracts/` | The unit schema and catalogs, and the JSON Schemas every model answer is checked against |

## Before you start

The exact toolchain versions are recorded in
[docs/ENVIRONMENT.md](docs/ENVIRONMENT.md): Rust, Cargo, Node, pnpm, the
browser used for the tests, and which machine each row was checked on. Read it
instead of copying versions into a script or into a document. It is the record
of what was actually run, and it changes with the pin in
`rust-toolchain.toml`.

Branch from `development` and merge back into `development`. `main` is stale and
is not used.

The pre-commit hook runs the whole CI set: the format, the lint, the
typecheck, every Rust test and every interface test, plus the build and the
licence checks. On the owner's machine that takes about eight minutes, which
surprises people. While iterating, use the short form:

```sh
LUMINGO_PRECOMMIT=quick git commit ...
```

That is an iteration shortcut, not a way to skip CI. CI runs everything on
every push.

## Build and test

[scripts/verify.sh](scripts/verify.sh) is the gate. It runs the checks CI runs,
on the machine that is committing, against the working tree. The commands it
runs, in order:

```sh
node scripts/check-case-collisions.mjs
node scripts/check-export-only.mjs
node scripts/check-docs.mjs
cargo fmt --all -- --check
node scripts/check-license-register.mjs
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git diff --quiet -- apps/web/src/generated
cargo deny check licenses
pnpm install --frozen-lockfile
pnpm --dir apps/web install --frozen-lockfile
pnpm --dir apps/web lint
pnpm --dir apps/web typecheck
pnpm --dir apps/web test
pnpm --dir apps/web build
pnpm --dir apps/web run check:licenses
```

Two lines are not plain commands. The `git diff --quiet -- apps/web/src/generated`
line is the check that the generated TypeScript types match what is committed,
and the `cargo deny check licenses` line runs only when `cargo deny` is
installed.

`LUMINGO_PRECOMMIT=quick` runs seven checks only:
`scripts/check-case-collisions.mjs`, `scripts/check-export-only.mjs`,
`scripts/check-docs.mjs`, `cargo fmt --all -- --check`,
`scripts/check-license-register.mjs`, `pnpm --dir apps/web lint` and
`pnpm --dir apps/web typecheck`. It is an iteration shortcut, not a way to
skip CI, and CI still runs everything. `git commit --no-verify` skips the hook;
CI still runs every check.

The browser suite, `pnpm test:e2e`, is manual. It never runs in the hook and
never runs in CI, so a green commit does not mean the browser suite passed.

## Commit rules

Conventional Commits, with the task id in the subject:

```text
feat(grammar): name the rule and say what it catches (S4-05)
docs(contributing): write the contributor guide (S7-06)
```

- Every commit is authored as the repository owner.
- No `Co-Authored-By` line, in any commit message.
- One commit per completed logical unit. Atomic commits, never one giant
  commit at the end.
- Never rebase published history, never force-push, never delete a branch.

## Do not commit

Never commit:

- `.env` or `providers.toml`. They hold the provider settings and the key.
  `.env.example` is the only file in that family that may be read and
  committed.
- Model weights and model files.
- Learner recordings.
- Downloaded datasets, and the fixtures under `benchmarks/fixtures/` that a
  script fetches.
- Anything from the untracked local trees: `.blueprint/`, `context_pack.md`,
  `HANDOFF.md`, and the crew worktrees under `.crew/`.

A third-party item needs a row in
[docs/LICENSE_REGISTER.md](docs/LICENSE_REGISTER.md) before it enters the build,
the release zip, the repository or a first-run download. A dependency's licence
is checked at the source — the licence file in the package, the model card, the
publisher's terms — before the dependency is added, and the row records the
status and the date it was checked.

The gate checks that the register and the build agree. It reads the
dependencies the build ships and fails when one of them has no row, when a
row's licence or status does not match the build, or when a row is for
something the build does not ship.

## Writing documentation

Every claim must be traceable to code, to a test, or to a recorded run. If a
claim was not checked, write that it was not checked.

PRD section 12.4 lists the wording that is allowed and the wording that is
banned, and it applies to documents as well as to the interface. The banned
column covers claims that the tutor needs no connection to work at all, claims
that the intelligence runs entirely on the device, claims that no remote
service is involved, a standalone claim of secrecy, claims of accreditation or
of a level that comes from an authority, comparisons with a well-known exam,
claims of exact pronunciation scoring, claims that the key or the learner data
is protected at rest, and calling the app a place on the internet or a service
that runs on someone else's machine.

The allowed column gives the phrasing to use instead:

- "Runs on your own computer and opens in your browser."
- "Your voice stays on your device."
- "Text is sent to the AI provider you choose."
- "Estimated level, based on your recorded work."
- "Pronunciation feedback is experimental."
- "Your API key is saved in plain text in a file on this computer."

The reason is one sentence: the app must not claim more than it does.

When a document states a limit, the limit is written in the open rather than
left out. The limits of the current build are written out in
[docs/LIMITATIONS.md](docs/LIMITATIONS.md).
