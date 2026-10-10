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
| axum | HTTP API and WebSocket in `apps/server`; scripted provider server in the `llm-client` tests | MIT | ok |
| tokio, tokio-util | Async runtime and cancellation. `storage` also uses its file functions for the pre-upgrade backup. | MIT | ok |
| tower, http-body-util, tokio-tungstenite, futures-util | Server tests | MIT | ok |
| rust-embed | Embeds the exported UI in the executable | MIT | ok |
| ts-rs | Generates TypeScript types from Rust API types, and from the unit types behind the `ts` feature of `curriculum` | MIT | ok |
| jsonschema | Checks unit files and catalogs against their JSON Schema in `crates/curriculum`. Default features are off, so it has no HTTP or file reference resolver and no TLS stack. Checked 2026-10-05, version 0.58.5. | MIT | ok |
| referencing, jsonschema-regex, jsonschema-value, fluent-uri, fancy-regex, email_address, uuid-simd, vsimd, outref, data-encoding, micromap, fraction, num-cmp, bytecount, ahash, strum, unicode-general-category | Dependencies of jsonschema. Checked 2026-10-05. | MIT, or MIT OR Apache-2.0, or Apache-2.0 (unicode-general-category) | ok |
| borrow-or-share | Dependency of fluent-uri, which jsonschema uses for references. Its LICENSE file is MIT No Attribution, which asks for less than MIT. It is allowed through a per-crate exception in `deny.toml`. Checked 2026-10-05, version 0.2.4. | MIT-0 | ok |
| serde, serde_json | Serialisation | MIT OR Apache-2.0 | ok |
| clap | Command-line options of `apps/server`, `tools/content-cli` and `tools/tutor-cli` | MIT OR Apache-2.0 | ok |
| anyhow, thiserror | Errors | MIT OR Apache-2.0 | ok |
| tracing, tracing-subscriber | Logging | MIT | ok |
| sha2, base64 | Content-Security-Policy script hashes. sha2 also gives the SHA-256 checksum of each unit file in `crates/curriculum`, and the checksums of the downloaded model files in `crates/model-manager`. A development dependency of `crates/app-core`, for the model download tests. | MIT OR Apache-2.0 | ok |
| getrandom | Session secret | MIT OR Apache-2.0 | ok |
| webbrowser | Opens the default browser | MIT OR Apache-2.0 | ok |
| sqlx (sqlx-core, sqlx-sqlite) | SQLite access, connection pools and migrations in `crates/storage`. Runtime query functions only, no compile-time query macros. | MIT OR Apache-2.0 | ok |
| libsqlite3-sys 0.37.0 and the bundled SQLite 3.51.3 | Builds SQLite from source inside `crates/storage` so no system library is needed | MIT (the crate). The bundled SQLite source states "The author disclaims copyright to this source code", which is public domain. | ok |
| chrono | `Timestamp` and `LocalDate` in `crates/storage`, built without the clock and time-zone features | MIT OR Apache-2.0 | ok |
| tempfile | One temporary database file per test in `crates/storage` (dev only) | MIT OR Apache-2.0 | ok |
| foldhash, ICU crates (`icu_*`, `idna`, `url`) and other small transitive crates of sqlx | Hashing, URL parsing | Zlib, Unicode-3.0, MIT OR Apache-2.0 | ok |
| futures-util | Streams in `llm-client` and the server tests | MIT OR Apache-2.0 | ok |
| async-trait | Async trait methods of `LlmClient` | MIT OR Apache-2.0 | ok |
| url | URL and host parsing for the allowlist | MIT OR Apache-2.0 | ok |
| reqwest | HTTP client for LLM calls in `llm-client`; built without default features, so no bundled TLS provider and no system proxy | MIT OR Apache-2.0 | ok |
| hyper, hyper-util, tokio-native-tls | Transport under reqwest | MIT | ok |
| hyper-tls | TLS connector under reqwest | MIT/Apache-2.0 | ok |
| native-tls | TLS backend choice: Schannel on Windows (no library to install), Security framework on macOS, OpenSSL on Linux | MIT OR Apache-2.0 | ok |
| schannel | Windows binding used by native-tls | MIT | ok |
| openssl, openssl-sys, openssl-probe | Linux binding used by native-tls. They link the system OpenSSL library and do not bundle it; building needs its headers | Apache-2.0, MIT, MIT OR Apache-2.0 | ok |
| toml (with toml_parser, toml_writer, toml_datetime, serde_spanned, winnow) | Reads and writes `providers.toml` in `llm-client`; reads the engines file (which model files the speech engines load) in `crates/app-core`, and the sherpa engines file of `tools/tutor-cli` (`sherpa` feature, off by default) | MIT OR Apache-2.0; winnow MIT | ok |
| tempfile (with fastrand) | Temporary directories in `llm-client` tests | MIT OR Apache-2.0 | ok |
| sysinfo | Installed memory in the hardware profile of `crates/app-core` (only the `system` feature; the same crate version as `tools/bench`). Checked 2026-10-05, version 0.36.1. | MIT | ok |
| iana-time-zone, iana-time-zone-haiku, android_system_properties | Pulled in by the `clock` feature of chrono, which `crates/app-core` uses to read the learner's local calendar day for streaks (unix targets only; Windows uses `windows-link`). Checked 2026-10-05. | MIT OR Apache-2.0 | ok |
| rtrb | Lock-free single-producer single-consumer ring buffer in `audio-io` (0.4.0). Its `unsafe` is inside the crate, audited by its authors; `audio-io` itself forbids unsafe. | MIT OR Apache-2.0 | ok |
| rubato 5.0.1 and its dependencies (audioadapter, audioadapter-buffers, audioadapter-sample, realfft, rustfft, windowfunctions, num-complex, num-integer, num-traits, primal-check, strength_reduce, transpose, visibility) | Sample-rate conversion to 16 kHz and to the playback device rate in `audio-io`. Licenses read from each crate's `Cargo.toml` in the registry source. | MIT OR Apache-2.0 for rubato; realfft and windowfunctions MIT; visibility Zlib OR MIT OR Apache-2.0; the rest MIT OR Apache-2.0 | ok |
| audio-codec-algorithms 0.8.1 | Sample-format conversion pulled in by audioadapter-sample | 0BSD OR Apache-2.0 (used under Apache-2.0) | ok |
| cpal 0.18.2 and its platform dependencies (windows, windows-core and siblings, alsa, alsa-sys, coreaudio-rs, objc2 family, jni, ndk, wasm-bindgen family, dasp_sample, and others) | Audio capture and playback in `audio-io`, behind the off-by-default `cpal-backend` feature. Licenses read from `cargo metadata --all-features` on 2026-10-05, every platform's dependencies included. UNVERIFIED on hardware. | cpal Apache-2.0; dependencies MIT, Apache-2.0, Zlib or BSD choices (all offer MIT or Apache-2.0) | ok |
| toml | Reads the phone map and the calibration file in `pron-engine` | MIT OR Apache-2.0 | ok |
| hound | Reads WAV files in the `pron` program (`ort-backend` feature, off by default) | Apache-2.0 | ok |
| ort, ort-sys 2.0.0-rc.13 | ONNX Runtime bindings for the phoneme model (`ort-backend` feature, off by default). Built with `load-dynamic`, so no runtime is downloaded or linked at build time. | MIT OR Apache-2.0 | ok. Read from the crate metadata and the `LICENSE-MIT` and `LICENSE-APACHE` files in the published package. The ONNX Runtime library the program loads is a separate item (section 6). |
| libloading | Loads the ONNX Runtime library (dependency of `ort`) | ISC | ok |
| ndarray, matrixmultiply, rawpointer, num-complex, num-integer, num-traits | Dependencies of `ort` | MIT OR Apache-2.0 | ok |
| webpki-roots | CA root data, pulled in only by the build script of `sherpa-onnx-sys` when the optional `sherpa` feature of `speech` is on. Not in the default build. | CDLA-Permissive-2.0 | review: per-crate exception in `deny.toml` for this reason only |

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
| husky, lint-staged | Git hooks at the repository root: lint-staged formats and fixes only the staged files, then `scripts/verify.sh` runs the checks of `.github/workflows/ci.yml` before a commit is made. Development tooling only; not part of the web app, the server, or the exported UI. | MIT | ok | Checked 2026-10-09. Pinned in the root `package.json`. |
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
| sherpa-onnx 1.13.8 (Rust crate and native library) | VAD, STT, TTS runtime, optional `sherpa` feature of `speech` | Apache-2.0 (crate metadata) | review: the build script of `sherpa-onnx-sys` 1.13.8 links the static libraries `espeak-ng` and `piper_phonemize` into every static build. espeak-ng is GPL-3.0 (section 7). Read the licence of the prebuilt archive and decide before the feature is enabled in a release. The shared-library mode was not inspected. |
| ONNX Runtime (the native library; the `ort` crate is in section 3) | Phoneme model inference. `pron-engine` loads it at run time from `ORT_DYLIB_PATH`; nothing in the repository bundles it. | MIT | verify |
| harper-core 0.68.0 (with harper-brill 0.68.0, harper-pos-utils 0.68.0, burn 0.18.0, ammonia 4.2.1, cssparser 0.38.0, dtoa-short 0.3.5, colored 3.1.1) | Rule-based grammar findings behind the `GrammarCheck` seam (assessment spec 5.3, X5; writing workshop first layer; warning W03). Licenses read from the `Cargo.toml` of each crate in the registry source on 2026-10-06 (0.54.0) and re-checked on 2026-10-10 (0.68.0): harper-core, harper-brill and harper-pos-utils say `Apache-2.0`, ammonia says `MIT OR Apache-2.0`, **cssparser, dtoa-short and colored say `MPL-2.0`**. harper-core needs ammonia (to clean markdown), ammonia needs cssparser, cssparser needs dtoa-short; colored is a dependency of the test/terminal paths of the harper stack. The package of harper-core holds no license file, and the upstream repository could not be reached to read one. The package also carries `dictionary.dict`, whose own terms the package does not state. **Version chosen empirically on 2026-10-10:** the pre-merge line linked 2.11.0, but that version (and 0.69+) cannot resolve in this tree — `harper-pos-utils` → `burn` → `tracel-llvm-bundler` → `liblzma-sys` links the native `lzma`, which conflicts with `sherpa-onnx-sys` → `zip` → `xz2` → `lzma-sys` (the pre-merge tree had no `speech/sherpa`, so the conflict never appeared there). 0.68.0 is the newest version that resolves next to sherpa (0.69 and 0.70 were tried and failed; 0.55..0.68 all resolve). It still pulls `burn` 0.18.0 (272 new lock entries) but no second native lzma link. `cargo deny check licenses` rejects exactly the three MPL-2.0 crates; the per-crate exceptions in `deny.toml` cover them. **A dependency of the workspace since 2026-10-10** (owner decision of that day, ADR-057): `assessment-engine`'s default-on `grammar` feature, linked into `tutor-engine` (X5, workshop layer one), `content-cli` (W03, spelling off) and `app-core` (one dictionary load in `Catalogs`). | Apache-2.0 for harper-core; MPL-2.0 for cssparser, dtoa-short and colored | ok (owner decision 2026-10-10). Weigh its build and memory cost in stage 7 measurements. |
| Silero VAD, Whisper, Moonshine (English), Parakeet | VAD and STT candidates | MIT, MIT, MIT, CC BY 4.0 | verify |
| Supertonic 3 | TTS candidate | Model: OpenRAIL-M. Code: MIT. | review: read the use restrictions in full and show them before download |
| Kokoro, Kitten TTS | TTS candidates | Apache-2.0 | verify |
| wav2vec2 phoneme model, ZIPA | Phoneme model candidates | Apache-2.0, unknown | verify |
| CMUdict | Canonical pronunciations. `pron-engine` ships no dictionary: it reads a `cmudict.dict` file the user supplies. Crates.io was checked on 2026-10-05: `cmudict-fast` 0.8.0 and `mora-cmudict` 0.0.1 carry a copy of the data with its terms in `LICENSE-CMUDICT` (copyright 1993-2015 Carnegie Mellon University, redistribution allowed if the notice is kept), but the data is not covered by the crates' own `MIT OR Apache-2.0` metadata and the original source could not be reached to compare, so neither is a dependency. Shipping the data would need a `NOTICE` line. | BSD-style | verify |
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

## 9. Promotional video

The promo video in `docs/` is built from this project's own interface and art,
with one third-party element: the voiceover and sound effects.

| Item | Use | License | Status | Notes |
|---|---|---|---|---|
| ElevenLabs-generated audio (short cut: voiceover, 5 lines, 6 effects. Long cut: voiceover, 10 lines, 6 effects) | Narration and sound effects in `docs/lumingo-promo.mp4`, `docs/lumingo-promo.webp` and `docs/lumingo-promo-long.mp4` | ElevenLabs terms of use; generated on the **Free** plan | review | Read at `https://elevenlabs.io/docs/help-center/legal/can-i-publish-the-content-i-generate-on-the-platform` on 2026-10-10 and re-checked 2026-10-11. The Free plan carries **no commercial licence** and requires attribution: published content must credit `elevenlabs.io`. The videos promote an Apache-2.0 project and are not sold, so the non-commercial limit is not breached, and the attribution requirement is met by the credit line under the embed in `README.md`. **If the project ever needs commercial use of this audio, or the credit line is removed, this row becomes blocking** — a paid plan grants the commercial licence and drops the attribution requirement. The audio is not part of the application, the release zip, or any first-run download; it exists only in the three committed video files. |

Both cuts are built from the same source. The long cut's captions use
word-level timings from the same alignment endpoint as the short cut's, and both
are built by the gitignored `.video/` project described below.

The video's own text is read at build time from `apps/web/src/i18n/en.ts`, so no
copy is duplicated. The fonts it uses are the bundled OFL faces in section 5.
The Remotion tooling that builds it lives in a gitignored `.video/` folder and is
**not** a dependency of this project: no Remotion package enters the build, the
release, or the repository. Remotion's licence (free for individuals and
companies of up to three people) therefore does not apply to this repository's
distribution, and no row is needed for it.

This section is deliberately last. Adding it as section 6 shifted every section
after it, and five cross-references in the repository point at sections by
number — `crates/speech/README.md`, `deny.toml`, `models/manifest.toml` and two
rows above all broke silently. New sections go at the end, or the references
need updating in the same commit.
