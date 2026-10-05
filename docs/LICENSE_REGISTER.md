# License register

Nothing third-party enters the build, the release zip, the repository, or a
first-run download without a row here. This file records what each item's
license states and how the project uses it. It is not legal advice. For
anything marked `review`, read the full license text before use.

## 1. Project licenses

| Part | License | File |
|---|---|---|
| Source code | Apache-2.0 | `LICENSE` |
| Curriculum content written for this project | CC BY-SA 4.0 | `LICENSE-CONTENT` |
| Artwork drawn for this project (Lumi, icons, favicon) | Apache-2.0 | `LICENSE` |
| Third-party data in `curriculum/data/` | Each under its own terms | A license file next to each data set |
| Speech models | Each under its own terms. Downloaded by the user's machine from the upstream host. Not part of the repository or the release zip. | `models/manifest.toml` |

## 2. Rules

1. **Status values.** `ok`: license known and compatible, checked on the date shown, against the package that was actually downloaded. `verify`: believed compatible, must be checked at the source before use. `review`: usable only under the conditions in the notes. `blocked`: must not be used.
2. **Code.** Allowed without discussion: MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC, Zlib, Unlicense, CC0-1.0, BSL-1.0, Unicode licenses. A row with status `review` is needed first for MPL-2.0 and LGPL. Blocked: GPL, AGPL, SSPL, and any license with a non-commercial or field-of-use limit on code.
3. **Models.** A model may carry use restrictions (for example OpenRAIL-M) if the project does not redistribute it, the app shows the license before download, and the restrictions do not conflict with normal use of a language tutor. Non-commercial model licenses are blocked.
4. **Content sources.** Authored content is original. Third-party text may enter authored files only if its license is compatible with CC BY-SA 4.0. Otherwise it stays in its own file under its own terms and is used as data.
5. **Evaluation data.** Datasets used only to measure are downloaded by script, never committed, and never shipped.
6. **Tooling.** `cargo deny check licenses` (configured in `deny.toml`) and `pnpm run check:licenses` (in `apps/web`) run in CI. Their allowlists mirror rule 2.
7. **Attribution.** `NOTICE` lists the attribution lines that the licenses below require.

## 3. Rust crates in the build

Licenses were read from the crate metadata of the versions in `Cargo.lock`, and
`cargo deny check licenses` passes. Checked 2026-10-05.

| Crate | Use | License | Status |
|---|---|---|---|
| axum | HTTP API and WebSocket in `apps/server` | MIT | ok |
| tokio, tokio-util | Async runtime and cancellation. `storage` also uses its file functions for the pre-upgrade backup. | MIT | ok |
| tower, http-body-util, tokio-tungstenite, futures-util | Server tests | MIT | ok |
| rust-embed | Embeds the exported UI in the executable | MIT | ok |
| ts-rs | Generates TypeScript types from Rust API types, and from the unit types behind the `ts` feature of `curriculum` | MIT | ok |
| jsonschema | Checks unit files and catalogs against their JSON Schema in `crates/curriculum`. Default features are off, so it has no HTTP or file reference resolver and no TLS stack. Checked 2026-10-05, version 0.58.5. | MIT | ok |
| referencing, jsonschema-regex, jsonschema-value, fluent-uri, fancy-regex, email_address, uuid-simd, vsimd, outref, data-encoding, micromap, fraction, num-cmp, bytecount, ahash, strum, unicode-general-category | Dependencies of jsonschema. Checked 2026-10-05. | MIT, or MIT OR Apache-2.0, or Apache-2.0 (unicode-general-category) | ok |
| borrow-or-share | Dependency of fluent-uri, which jsonschema uses for references. Its LICENSE file is MIT No Attribution, which asks for less than MIT. It is allowed through a per-crate exception in `deny.toml`. Checked 2026-10-05, version 0.2.4. | MIT-0 | ok |
| serde, serde_json | Serialisation | MIT OR Apache-2.0 | ok |
| clap | Command-line options of `apps/server` and `tools/content-cli` | MIT OR Apache-2.0 | ok |
| anyhow, thiserror | Errors | MIT OR Apache-2.0 | ok |
| tracing, tracing-subscriber | Logging | MIT | ok |
| sha2, base64 | Content-Security-Policy script hashes. sha2 also gives the SHA-256 checksum of each unit file in `crates/curriculum`. | MIT OR Apache-2.0 | ok |
| getrandom | Session secret | MIT OR Apache-2.0 | ok |
| webbrowser | Opens the default browser | MIT OR Apache-2.0 | ok |
| sqlx (sqlx-core, sqlx-sqlite) | SQLite access, connection pools and migrations in `crates/storage`. Runtime query functions only, no compile-time query macros. | MIT OR Apache-2.0 | ok |
| libsqlite3-sys 0.37.0 and the bundled SQLite 3.51.3 | Builds SQLite from source inside `crates/storage` so no system library is needed | MIT (the crate). The bundled SQLite source states "The author disclaims copyright to this source code", which is public domain. | ok |
| chrono | `Timestamp` and `LocalDate` in `crates/storage`, built without the clock and time-zone features | MIT OR Apache-2.0 | ok |
| tempfile | One temporary database file per test in `crates/storage` (dev only) | MIT OR Apache-2.0 | ok |
| foldhash, ICU crates (`icu_*`, `idna`, `url`) and other small transitive crates of sqlx | Hashing, URL parsing | Zlib, Unicode-3.0, MIT OR Apache-2.0 | ok |

## 4. JavaScript packages

Licenses were read from each installed package's `package.json`. Checked 2026-10-05.

| Package | Use | License | Status | Notes |
|---|---|---|---|---|
| next | UI framework, static export only | MIT | ok | Build telemetry is switched off in the scripts and in CI. |
| react, react-dom | UI | MIT | ok | |
| typescript | Language | Apache-2.0 | ok | Pinned to 5.9 because typescript-eslint does not support 7 yet. |
| eslint, eslint-config-next | Lint | MIT | ok | |
| vitest, jsdom, @testing-library/* , @vitejs/plugin-react | Tests | MIT | ok | |
| cross-env | Sets environment variables in scripts on every OS | MIT | ok | |
| caniuse-lite | Browser data used by the Next.js build | CC-BY-4.0 | ok | Data, not shipped in the UI. Attribution is in `NOTICE`. |
| sharp and its libvips binaries | Optional Next.js image optimiser | Apache-2.0 and LGPL-3.0-or-later | excluded | Dropped with `pnpm.ignoredOptionalDependencies`. The UI uses unoptimised images only. |

Development-only tools with MPL-2.0, BlueOak-1.0.0 or Python-2.0 licenses may
appear in the full tree. None of them reaches the exported UI. The list is
checked by `apps/web/scripts/check-licenses.mjs`.

## 5. Fonts

Bundled as files in `apps/web/public/fonts/`. Nothing is loaded from a font
service. The OFL text for each font is in `apps/web/public/fonts/licenses/`
and is served by the app next to the fonts. Source: the `@fontsource` packages,
version 5.3.0, whose license files were read on 2026-10-05.

| Font | Use | License | Status | Notes |
|---|---|---|---|---|
| Press Start 2P | Titles, labels, numbers | OFL-1.1 | ok | Reserved Font Name "Press Start 2P". Files are the upstream design split into Unicode subsets, with no change to the glyphs. |
| Pixelify Sans | Body text and reading text | OFL-1.1 | ok | |
| Noto Sans Mono | Phoneme symbols (the only bundled font with IPA glyphs) | OFL-1.1 | ok | |

## 6. Planned, not yet in the build

These rows are carried over from the project plan. None is used yet, and none
has been checked at the source, because the build container cannot reach the
upstream hosts. Each stays `verify` until someone reads the license at the source.

| Item | Planned use | Expected license | Status |
|---|---|---|---|
| sherpa-onnx (Rust crate and native library) | VAD, STT, TTS runtime | Apache-2.0 | verify |
| ONNX Runtime, `ort` | Phoneme model inference | MIT, MIT OR Apache-2.0 | verify |
| CPAL | Audio capture and playback | Apache-2.0 | verify |
| Rubato, a ring-buffer crate | Resampling, audio buffers | MIT | verify |
| reqwest | LLM HTTP client | MIT OR Apache-2.0 | verify |
| jsonschema | Local validation of structured output and units | MIT | verify |
| harper-core | Rule-based grammar findings | Apache-2.0 | verify |
| Silero VAD, Whisper, Moonshine (English), Parakeet | VAD and STT candidates | MIT, MIT, MIT, CC BY 4.0 | verify |
| Supertonic 3 | TTS candidate | Model: OpenRAIL-M. Code: MIT. | review: read the use restrictions in full and show them before download |
| Kokoro, Kitten TTS | TTS candidates | Apache-2.0 | verify |
| wav2vec2 phoneme model, ZIPA | Phoneme model candidates | Apache-2.0, unknown | verify |
| CMUdict | Canonical pronunciations | BSD-style | verify |
| CEFR-J vocabulary profile | Checking word levels | Free with citation, not an open license | review |
| Octanove vocabulary profile C1/C2 | Word levels above B2 | CC BY-SA 4.0 | verify |
| speechocean762 | Evaluation only | CC BY 4.0 | verify |

## 7. Excluded on purpose

| Item | Reason |
|---|---|
| espeak-ng, piper1-gpl | GPL-3.0. Linking them would force the app's distribution under GPL. |
| Moonshine models for languages other than English | Non-commercial license. |
| L2-ARCTIC | Non-commercial. |
| CEFR and CEFR Companion Volume descriptor text | Council of Europe copyright. Scale names and citations only. |
| English Vocabulary Profile, English Grammar Profile, Oxford 3000 and 5000, textbook or exam text | Not openly licensed. |
| llama.cpp and any local language model | Out of scope for the product. |
| LanguageTool | Needs a Java runtime. Too heavy for the target machine. |

## 8. How to add an item

1. Find the license at the source: the license file in the package, the model card, or the publisher's terms page. A registry's metadata is a hint, not proof.
2. Add a row with a status and today's date.
3. If the status is `review` or `blocked`, stop and ask the owner.
4. If the item is a model, put its license and license URL in `models/manifest.toml` too.
5. If the license needs attribution, add the line to `NOTICE`.
