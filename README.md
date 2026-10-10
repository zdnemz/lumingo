# Lumingo

An open-source English tutor for A1 to C2. It trains listening, speaking,
reading, and writing, with a pixel-art game interface.

Lumingo runs on your own computer and opens in your browser. One Rust program
owns the microphone, the speakers, the speech models, and your data, and it
serves the interface on the loopback address.

[![A 31-second tour of Lumingo: the quest map, an activity, and what stays on your machine.](docs/lumingo-promo.webp)](docs/lumingo-promo.mp4)

*31 seconds. [Full-quality MP4](docs/lumingo-promo.mp4) — the animation above is
a smaller copy for this page. Voiceover and sound effects generated with
[elevenlabs.io](https://elevenlabs.io); see `docs/LICENSE_REGISTER.md`.*

## Status

Early development. Nothing in this repository has been measured yet, and no
performance number is claimed. Features are listed here only when they work.

## What stays where

- Your voice stays on your device. Audio never goes to the browser or the network.
- Text is sent to the AI provider you choose, with your own key and base URL.
- Your API key is saved in plain text in a file on this computer, or read from `.env`.
- There is no telemetry and no update check.

## Estimates, not certificates

Level estimates are computed on your computer from your recorded work and are
always labelled as estimates. Pronunciation feedback is experimental.

## Configuration

Copy `.env.example` to `.env` and fill in the four values:

```
TUTOR_LLM_PROTOCOL=openai_chat        # or anthropic_messages
TUTOR_LLM_BASE_URL=
TUTOR_LLM_MODEL=
TUTOR_LLM_API_KEY=
```

Never commit `.env`.

## Development

The pre-commit hook (husky and lint-staged) formats the staged files and then
runs the same checks as CI. After cloning, install the hook tooling once:

```
pnpm install
```

`sh scripts/verify.sh` runs the checks on demand. While iterating,
`LUMINGO_PRECOMMIT=quick git commit ...` keeps only the path case check, format,
lint, and typecheck. `git commit --no-verify` skips the hook; CI still runs
every check.

## Licenses

- Code: Apache-2.0 (`LICENSE`).
- Curriculum content written for this project: CC BY-SA 4.0 (`LICENSE-CONTENT`).
- Third-party items and their terms: `docs/LICENSE_REGISTER.md`.
