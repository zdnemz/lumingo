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

1. **Status values.** `ok`: license known and compatible, checked on the date shown. In the prose sections 3 to 7 that check is a human one: someone read the license text at the source, against the package that was actually downloaded. In the machine table of section 10 the check is a machine one, and it means exactly this: the license is the one the package's own metadata states, and it is on the allow list that `cargo deny` and `check-licenses.mjs` enforce, as of the date shown. `verify`: believed compatible, must be checked at the source before use. `review`: usable only under the conditions in the notes. `blocked`: must not be used.
2. **Code.** Allowed without discussion: MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC, Zlib, Unlicense, CC0-1.0, BSL-1.0, Unicode licenses. A row with status `review` is needed first for MPL-2.0 and LGPL. Blocked: GPL, AGPL, SSPL, and any license with a non-commercial or field-of-use limit on code.
3. **Models.** A model may carry use restrictions (for example OpenRAIL-M) if the project does not redistribute it, the app shows the license before download, and the restrictions do not conflict with normal use of a language tutor. Non-commercial model licenses are blocked.
4. **Content sources.** Authored content is original. Third-party text may enter authored files only if its license is compatible with CC BY-SA 4.0. Otherwise it stays in its own file under its own terms and is used as data.
5. **Evaluation data.** Datasets used only to measure are downloaded by script, never committed, and never shipped.
6. **Tooling.** `cargo deny check licenses` (configured in `deny.toml`) and `pnpm run check:licenses` (in `apps/web`) run in CI. Their allowlists mirror rule 2.
7. **Attribution.** `NOTICE` lists the attribution lines that the licenses below require.
8. **Notices.** Two licences in the shipped set need a notice of their own: `MPL-2.0` and `CC-BY-4.0`. Every shipped item whose licence is one of those has a row in the attribution table of section 10, and `scripts/gen-notices.mjs` renders that row's notice under the item's own line in `NOTICE`.

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
| e2e, @e2e-dev/web | Browser end-to-end tests at the repository root (`pnpm test:e2e`): drives the built UI in a real Chromium against a locally started server. Run manually, outside the pre-commit hook and CI. Development tooling only; not part of the web app, the server, or the exported UI. | Apache-2.0 | ok | Checked 2026-10-13. Pinned in the root `package.json`. |
| ai, @ai-sdk/anthropic, @ai-sdk/google, @ai-sdk/openai, @ai-sdk/openai-compatible, zod | The model driver of the e2e harness, used only by its optional agent-driven tests (the default offline suite does not use it). Development tooling only. | Apache-2.0 (zod: MIT) | ok | Checked 2026-10-13. Pinned in the root `package.json`. |
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
6. A new row in the machine table of section 10 is not written by hand: run `scripts/gen-notices.mjs --print` to get the rows the build now has, read the license of each new item at the source, and keep the row's status `ok` only when the license is known and compatible. An item whose license is not known yet gets no `ok` row — it goes to the owner first, as step 3 says.

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

This section is not the last one any more: section 10 came after it. The rule
is the same as it was. Adding a section in the middle shifted every section
after it, and five cross-references in the repository point at sections by
number — `crates/speech/README.md`, `deny.toml`, `models/manifest.toml` and two
rows above all broke silently. New sections go at the end, or the references
need updating in the same commit.

## 10. The shipped set

The shipped set is what the Windows x64 default-feature build actually carries:
the `lumingo.exe` the release zip holds. It is not the whole of `Cargo.lock` —
the lock file and `cargo metadata`'s resolve graph carry roughly two hundred
further crates that the default build never activates (the off-by-default
`sherpa`, `ort` and `cpal-backend` stacks, and the dev-dependencies), and those
keep their rows in the earlier sections with their own statuses:
`webpki-roots` stays `review`, section 6's `verify` rows stay `verify`, and every
`review` row stays `review`. Two of them are repo assets rather than build
items — the promo video's audio in section 9 and the CEFR-J vocabulary profile
in section 6 — and neither gets a row here. Section 3's sentence about reading
the licenses from the crate metadata of the versions in `Cargo.lock` stays
true, and this section does not contradict it: this section says which of those
crates the default build links. Two groups are outside the set and get no row
here: the 15 workspace crates are the project itself, and the project's own
license is stated on its own in section 1; and the `@next/swc-*` compiler
binaries are Next.js's native compiler, one optional package per platform, and
they are build-time tooling that never reaches the exported UI or the release
zip. The set is derived five ways: the crates by
`cargo tree --edges normal,build --prefix none --format "{p} {l}" --target x86_64-pc-windows-msvc`,
the packages by `pnpm licenses list --prod --json` run in `apps/web`
(`pnpm --dir apps/web licenses list --prod --json`), minus the `@next/swc-*`
family; the fonts by the families in `apps/web/src/styles/fonts.css`, matched
to the OFL texts in `apps/web/public/fonts/licenses/`; the models by
`models/manifest.toml`; and the release files by the `members` array in
`scripts/package-release.mjs`. The target is pinned in the crate command, so
the set is the same on the Linux development machine and on the Windows runner.
Today that is 345 third-party crates, 51 packages, 3 font families, 0
downloadable models and 5 release members: 404 components in all, every one of
them rendered on the About page and listed in the `NOTICE`. The check proves
the register-row dimension (NFR-L1) — that this table agrees with the build,
and that the generated artifacts are complete — and it does **not** check
licence compatibility: that is `cargo deny check licenses` for the Rust crates
and `apps/web/scripts/check-licenses.mjs` for the JavaScript packages, both
already wired into the gate. The check does not restate their allow list and
does not claim that dimension.

The table has one row per shipped item, sorted by kind and then by item. The
columns are `Item`, `Kind`, `Licence` and `Status`. `Kind` is one of `crate`,
`package`, `font`, `model` or `release`. `Status` is `ok`, the only value the
check accepts for a shipped item, and rule 1 says what `ok` means for this
table: the licence is the one the package's own metadata states, and it is on
the allow list that `cargo deny` and `check-licenses.mjs` enforce, as of the
date shown. A dependency that enters the build gets one new row, and a
dependency that leaves gets its row deleted. A row's licence is the licence
the source states, not a paraphrase of it.

The rows are not written by hand. `scripts/gen-notices.mjs` writes them, and
`scripts/check-license-register.mjs` checks them.

Checked 2026-10-21.

| Item | Kind | Licence | Status |
|---|---|---|---|
| adler2 | crate | 0BSD OR MIT OR Apache-2.0 | ok |
| ahash | crate | MIT OR Apache-2.0 | ok |
| aho-corasick | crate | Unlicense OR MIT | ok |
| allocator-api2 | crate | MIT OR Apache-2.0 | ok |
| ammonia | crate | MIT OR Apache-2.0 | ok |
| anstream | crate | MIT OR Apache-2.0 | ok |
| anstyle | crate | MIT OR Apache-2.0 | ok |
| anstyle-parse | crate | MIT OR Apache-2.0 | ok |
| anstyle-query | crate | MIT OR Apache-2.0 | ok |
| anstyle-wincon | crate | MIT OR Apache-2.0 | ok |
| anyhow | crate | MIT OR Apache-2.0 | ok |
| async-trait | crate | MIT OR Apache-2.0 | ok |
| atoi | crate | MIT | ok |
| atomic-waker | crate | Apache-2.0 OR MIT | ok |
| atomic_float | crate | Apache-2.0 OR MIT OR Unlicense | ok |
| audio-codec-algorithms | crate | 0BSD OR Apache-2.0 | ok |
| audioadapter | crate | MIT OR Apache-2.0 | ok |
| audioadapter-buffers | crate | MIT OR Apache-2.0 | ok |
| audioadapter-sample | crate | MIT OR Apache-2.0 | ok |
| autocfg | crate | Apache-2.0 OR MIT | ok |
| axum | crate | MIT | ok |
| axum-core | crate | MIT | ok |
| base64 | crate | MIT OR Apache-2.0 | ok |
| bincode | crate | MIT | ok |
| bit-set | crate | Apache-2.0 OR MIT | ok |
| bit-vec | crate | Apache-2.0 OR MIT | ok |
| bitflags | crate | MIT OR Apache-2.0 | ok |
| blanket | crate | MIT | ok |
| block-buffer | crate | MIT OR Apache-2.0 | ok |
| borrow-or-share | crate | MIT-0 | ok |
| burn | crate | MIT OR Apache-2.0 | ok |
| burn-common | crate | MIT OR Apache-2.0 | ok |
| burn-core | crate | MIT OR Apache-2.0 | ok |
| burn-derive | crate | MIT OR Apache-2.0 | ok |
| burn-ir | crate | MIT OR Apache-2.0 | ok |
| burn-ndarray | crate | MIT OR Apache-2.0 | ok |
| burn-tensor | crate | MIT OR Apache-2.0 | ok |
| bytecount | crate | Apache-2.0/MIT | ok |
| bytemuck | crate | Zlib OR Apache-2.0 OR MIT | ok |
| bytemuck_derive | crate | Zlib OR Apache-2.0 OR MIT | ok |
| bytes | crate | MIT | ok |
| cached | crate | MIT | ok |
| cached_proc_macro | crate | MIT | ok |
| cached_proc_macro_types | crate | MIT | ok |
| cc | crate | MIT OR Apache-2.0 | ok |
| cfg-if | crate | MIT OR Apache-2.0 | ok |
| chrono | crate | MIT OR Apache-2.0 | ok |
| clap | crate | MIT OR Apache-2.0 | ok |
| clap_builder | crate | MIT OR Apache-2.0 | ok |
| clap_derive | crate | MIT OR Apache-2.0 | ok |
| clap_lex | crate | MIT OR Apache-2.0 | ok |
| colorchoice | crate | MIT OR Apache-2.0 | ok |
| colored | crate | MPL-2.0 | ok |
| const-oid | crate | Apache-2.0 OR MIT | ok |
| convert_case | crate | MIT | ok |
| cpufeatures | crate | MIT OR Apache-2.0 | ok |
| crc | crate | MIT OR Apache-2.0 | ok |
| crc-catalog | crate | MIT OR Apache-2.0 | ok |
| crc32fast | crate | MIT OR Apache-2.0 | ok |
| crossbeam-queue | crate | MIT OR Apache-2.0 | ok |
| crossbeam-utils | crate | MIT OR Apache-2.0 | ok |
| crypto-common | crate | MIT OR Apache-2.0 | ok |
| cssparser | crate | MPL-2.0 | ok |
| cubecl-common | crate | MIT OR Apache-2.0 | ok |
| darling | crate | MIT | ok |
| darling_core | crate | MIT | ok |
| darling_macro | crate | MIT | ok |
| data-encoding | crate | MIT | ok |
| deranged | crate | MIT OR Apache-2.0 | ok |
| derive-new | crate | MIT | ok |
| derive_more | crate | MIT | ok |
| derive_more-impl | crate | MIT | ok |
| digest | crate | MIT OR Apache-2.0 | ok |
| displaydoc | crate | MIT OR Apache-2.0 | ok |
| dtoa | crate | MIT OR Apache-2.0 | ok |
| dtoa-short | crate | MPL-2.0 | ok |
| either | crate | MIT OR Apache-2.0 | ok |
| email_address | crate | MIT | ok |
| embassy-futures | crate | MIT OR Apache-2.0 | ok |
| equivalent | crate | Apache-2.0 OR MIT | ok |
| event-listener | crate | Apache-2.0 OR MIT | ok |
| fancy-regex | crate | MIT | ok |
| fastrand | crate | Apache-2.0 OR MIT | ok |
| find-msvc-tools | crate | MIT OR Apache-2.0 | ok |
| flate2 | crate | MIT OR Apache-2.0 | ok |
| fluent-uri | crate | MIT | ok |
| flume | crate | Apache-2.0/MIT | ok |
| fnv | crate | Apache-2.0 / MIT | ok |
| foldhash | crate | Zlib | ok |
| form_urlencoded | crate | MIT OR Apache-2.0 | ok |
| fraction | crate | MIT OR Apache-2.0 | ok |
| fst | crate | Unlicense/MIT | ok |
| futures-channel | crate | MIT OR Apache-2.0 | ok |
| futures-core | crate | MIT OR Apache-2.0 | ok |
| futures-executor | crate | MIT OR Apache-2.0 | ok |
| futures-intrusive | crate | MIT OR Apache-2.0 | ok |
| futures-io | crate | MIT OR Apache-2.0 | ok |
| futures-lite | crate | Apache-2.0 OR MIT | ok |
| futures-macro | crate | MIT OR Apache-2.0 | ok |
| futures-sink | crate | MIT OR Apache-2.0 | ok |
| futures-task | crate | MIT OR Apache-2.0 | ok |
| futures-util | crate | MIT OR Apache-2.0 | ok |
| generic-array | crate | MIT | ok |
| getopts | crate | MIT OR Apache-2.0 | ok |
| getrandom | crate | MIT OR Apache-2.0 | ok |
| half | crate | MIT OR Apache-2.0 | ok |
| harper-brill | crate | Apache-2.0 | ok |
| harper-core | crate | Apache-2.0 | ok |
| harper-pos-utils | crate | Apache-2.0 | ok |
| hashbrown | crate | MIT OR Apache-2.0 | ok |
| hashlink | crate | MIT OR Apache-2.0 | ok |
| heck | crate | MIT OR Apache-2.0 | ok |
| html5ever | crate | MIT OR Apache-2.0 | ok |
| http | crate | MIT OR Apache-2.0 | ok |
| http-body | crate | MIT | ok |
| http-body-util | crate | MIT | ok |
| httparse | crate | MIT OR Apache-2.0 | ok |
| httpdate | crate | MIT OR Apache-2.0 | ok |
| hybrid-array | crate | MIT OR Apache-2.0 | ok |
| hyper | crate | MIT | ok |
| hyper-tls | crate | MIT/Apache-2.0 | ok |
| hyper-util | crate | MIT | ok |
| icu_collections | crate | Unicode-3.0 | ok |
| icu_locale_core | crate | Unicode-3.0 | ok |
| icu_normalizer | crate | Unicode-3.0 | ok |
| icu_normalizer_data | crate | Unicode-3.0 | ok |
| icu_properties | crate | Unicode-3.0 | ok |
| icu_properties_data | crate | Unicode-3.0 | ok |
| icu_provider | crate | Unicode-3.0 | ok |
| ident_case | crate | MIT/Apache-2.0 | ok |
| idna | crate | MIT OR Apache-2.0 | ok |
| idna_adapter | crate | Apache-2.0 OR MIT | ok |
| indexmap | crate | Apache-2.0 OR MIT | ok |
| ipnet | crate | MIT OR Apache-2.0 | ok |
| is-macro | crate | Apache-2.0 | ok |
| is_terminal_polyfill | crate | MIT OR Apache-2.0 | ok |
| itertools | crate | MIT OR Apache-2.0 | ok |
| itoa | crate | MIT OR Apache-2.0 | ok |
| jsonschema | crate | MIT | ok |
| jsonschema-regex | crate | MIT | ok |
| jsonschema-value | crate | MIT | ok |
| lazy_static | crate | MIT OR Apache-2.0 | ok |
| levenshtein_automata | crate | MIT | ok |
| libc | crate | MIT OR Apache-2.0 | ok |
| libm | crate | MIT | ok |
| libsqlite3-sys | crate | MIT | ok |
| litemap | crate | Unicode-3.0 | ok |
| lock_api | crate | MIT OR Apache-2.0 | ok |
| log | crate | MIT OR Apache-2.0 | ok |
| lru | crate | MIT | ok |
| maplit | crate | MIT/Apache-2.0 | ok |
| markup5ever | crate | MIT OR Apache-2.0 | ok |
| matchers | crate | MIT | ok |
| matchit | crate | MIT AND BSD-3-Clause | ok |
| matrixmultiply | crate | MIT/Apache-2.0 | ok |
| memchr | crate | Unlicense OR MIT | ok |
| micromap | crate | MIT | ok |
| mime | crate | MIT OR Apache-2.0 | ok |
| mime_guess | crate | MIT | ok |
| miniz_oxide | crate | MIT OR Zlib OR Apache-2.0 | ok |
| mio | crate | MIT | ok |
| native-tls | crate | MIT OR Apache-2.0 | ok |
| ndarray | crate | MIT OR Apache-2.0 | ok |
| new_debug_unreachable | crate | MIT | ok |
| ntapi | crate | Apache-2.0 OR MIT | ok |
| nu-ansi-term | crate | MIT | ok |
| num | crate | MIT OR Apache-2.0 | ok |
| num-bigint | crate | MIT OR Apache-2.0 | ok |
| num-cmp | crate | MIT/Apache-2.0 | ok |
| num-complex | crate | MIT OR Apache-2.0 | ok |
| num-conv | crate | MIT OR Apache-2.0 | ok |
| num-integer | crate | MIT OR Apache-2.0 | ok |
| num-iter | crate | MIT OR Apache-2.0 | ok |
| num-rational | crate | MIT OR Apache-2.0 | ok |
| num-traits | crate | MIT OR Apache-2.0 | ok |
| once_cell | crate | MIT OR Apache-2.0 | ok |
| once_cell_polyfill | crate | MIT OR Apache-2.0 | ok |
| ordered-float | crate | MIT | ok |
| outref | crate | MIT | ok |
| parking | crate | Apache-2.0 OR MIT | ok |
| parking_lot | crate | MIT OR Apache-2.0 | ok |
| parking_lot_core | crate | MIT OR Apache-2.0 | ok |
| paste | crate | MIT OR Apache-2.0 | ok |
| percent-encoding | crate | MIT OR Apache-2.0 | ok |
| phf | crate | MIT | ok |
| phf_codegen | crate | MIT | ok |
| phf_generator | crate | MIT | ok |
| phf_shared | crate | MIT | ok |
| pin-project-lite | crate | Apache-2.0 OR MIT | ok |
| pkg-config | crate | MIT OR Apache-2.0 | ok |
| portable-atomic | crate | Apache-2.0 OR MIT | ok |
| potential_utf | crate | Unicode-3.0 | ok |
| powerfmt | crate | MIT OR Apache-2.0 | ok |
| ppv-lite86 | crate | MIT OR Apache-2.0 | ok |
| precomputed-hash | crate | MIT | ok |
| primal-check | crate | MIT OR Apache-2.0 | ok |
| proc-macro2 | crate | MIT OR Apache-2.0 | ok |
| pulldown-cmark | crate | MIT | ok |
| pulldown-cmark-escape | crate | MIT | ok |
| quote | crate | MIT OR Apache-2.0 | ok |
| rand | crate | MIT OR Apache-2.0 | ok |
| rand_chacha | crate | MIT OR Apache-2.0 | ok |
| rand_core | crate | MIT OR Apache-2.0 | ok |
| rand_distr | crate | MIT OR Apache-2.0 | ok |
| rawpointer | crate | MIT/Apache-2.0 | ok |
| realfft | crate | MIT | ok |
| ref-cast | crate | MIT OR Apache-2.0 | ok |
| ref-cast-impl | crate | MIT OR Apache-2.0 | ok |
| referencing | crate | MIT | ok |
| regex | crate | MIT OR Apache-2.0 | ok |
| regex-automata | crate | MIT OR Apache-2.0 | ok |
| regex-syntax | crate | MIT OR Apache-2.0 | ok |
| reqwest | crate | MIT OR Apache-2.0 | ok |
| rmp | crate | MIT | ok |
| rmp-serde | crate | MIT | ok |
| rs-conllu | crate | MIT OR Apache-2.0 | ok |
| rtrb | crate | MIT OR Apache-2.0 | ok |
| rubato | crate | MIT OR Apache-2.0 | ok |
| rust-embed | crate | MIT | ok |
| rust-embed-impl | crate | MIT | ok |
| rust-embed-utils | crate | MIT | ok |
| rustc_version | crate | MIT OR Apache-2.0 | ok |
| rustfft | crate | MIT OR Apache-2.0 | ok |
| rustls-pki-types | crate | MIT OR Apache-2.0 | ok |
| ryu | crate | Apache-2.0 OR BSL-1.0 | ok |
| same-file | crate | Unlicense/MIT | ok |
| schannel | crate | MIT | ok |
| scopeguard | crate | MIT OR Apache-2.0 | ok |
| semver | crate | MIT OR Apache-2.0 | ok |
| serde | crate | MIT OR Apache-2.0 | ok |
| serde_bytes | crate | MIT OR Apache-2.0 | ok |
| serde_core | crate | MIT OR Apache-2.0 | ok |
| serde_derive | crate | MIT OR Apache-2.0 | ok |
| serde_json | crate | MIT OR Apache-2.0 | ok |
| serde_path_to_error | crate | MIT OR Apache-2.0 | ok |
| serde_spanned | crate | MIT OR Apache-2.0 | ok |
| serde_urlencoded | crate | MIT/Apache-2.0 | ok |
| sha1 | crate | MIT OR Apache-2.0 | ok |
| sha2 | crate | MIT OR Apache-2.0 | ok |
| sharded-slab | crate | MIT | ok |
| shlex | crate | MIT OR Apache-2.0 | ok |
| simd-adler32 | crate | MIT | ok |
| siphasher | crate | MIT OR Apache-2.0 | ok |
| slab | crate | MIT | ok |
| smallvec | crate | MIT OR Apache-2.0 | ok |
| socket2 | crate | MIT OR Apache-2.0 | ok |
| spin | crate | MIT | ok |
| sqlx | crate | MIT OR Apache-2.0 | ok |
| sqlx-core | crate | MIT OR Apache-2.0 | ok |
| sqlx-sqlite | crate | MIT OR Apache-2.0 | ok |
| stable_deref_trait | crate | MIT OR Apache-2.0 | ok |
| strength_reduce | crate | MIT OR Apache-2.0 | ok |
| string_cache | crate | MIT OR Apache-2.0 | ok |
| string_cache_codegen | crate | MIT OR Apache-2.0 | ok |
| strsim | crate | MIT | ok |
| strum | crate | MIT | ok |
| strum_macros | crate | MIT | ok |
| syn | crate | MIT OR Apache-2.0 | ok |
| sync_wrapper | crate | Apache-2.0 | ok |
| synstructure | crate | MIT | ok |
| sysinfo | crate | MIT | ok |
| tendril | crate | MIT OR Apache-2.0 | ok |
| termcolor | crate | Unlicense OR MIT | ok |
| thiserror | crate | MIT OR Apache-2.0 | ok |
| thiserror-impl | crate | MIT OR Apache-2.0 | ok |
| thread_local | crate | MIT OR Apache-2.0 | ok |
| time | crate | MIT OR Apache-2.0 | ok |
| time-core | crate | MIT OR Apache-2.0 | ok |
| tinystr | crate | Unicode-3.0 | ok |
| tokio | crate | MIT | ok |
| tokio-macros | crate | MIT | ok |
| tokio-native-tls | crate | MIT | ok |
| tokio-stream | crate | MIT | ok |
| tokio-tungstenite | crate | MIT | ok |
| tokio-util | crate | MIT | ok |
| toml | crate | MIT OR Apache-2.0 | ok |
| toml_datetime | crate | MIT OR Apache-2.0 | ok |
| toml_parser | crate | MIT OR Apache-2.0 | ok |
| toml_writer | crate | MIT OR Apache-2.0 | ok |
| tower | crate | MIT | ok |
| tower-http | crate | MIT | ok |
| tower-layer | crate | MIT | ok |
| tower-service | crate | MIT | ok |
| tracing | crate | MIT | ok |
| tracing-attributes | crate | MIT | ok |
| tracing-core | crate | MIT | ok |
| tracing-log | crate | MIT | ok |
| tracing-subscriber | crate | MIT | ok |
| transpose | crate | MIT OR Apache-2.0 | ok |
| try-lock | crate | MIT | ok |
| ts-rs | crate | MIT | ok |
| ts-rs-macros | crate | MIT | ok |
| tungstenite | crate | MIT OR Apache-2.0 | ok |
| typenum | crate | MIT OR Apache-2.0 | ok |
| unicase | crate | MIT OR Apache-2.0 | ok |
| unicode-blocks | crate | MIT | ok |
| unicode-general-category | crate | Apache-2.0 | ok |
| unicode-ident | crate | (MIT OR Apache-2.0) AND Unicode-3.0 | ok |
| unicode-script | crate | MIT OR Apache-2.0 | ok |
| unicode-segmentation | crate | MIT OR Apache-2.0 | ok |
| unicode-width | crate | MIT OR Apache-2.0 | ok |
| unicode-xid | crate | MIT OR Apache-2.0 | ok |
| unty | crate | MIT OR Apache-2.0 | ok |
| url | crate | MIT OR Apache-2.0 | ok |
| utf8_iter | crate | Apache-2.0 OR MIT | ok |
| utf8parse | crate | Apache-2.0 OR MIT | ok |
| uuid | crate | Apache-2.0 OR MIT | ok |
| uuid-simd | crate | MIT | ok |
| vcpkg | crate | MIT/Apache-2.0 | ok |
| version_check | crate | MIT/Apache-2.0 | ok |
| visibility | crate | Zlib OR MIT OR Apache-2.0 | ok |
| vsimd | crate | MIT | ok |
| walkdir | crate | Unlicense/MIT | ok |
| want | crate | MIT | ok |
| web-time | crate | MIT OR Apache-2.0 | ok |
| web_atoms | crate | MIT OR Apache-2.0 | ok |
| webbrowser | crate | MIT OR Apache-2.0 | ok |
| winapi | crate | MIT/Apache-2.0 | ok |
| winapi-util | crate | Unlicense OR MIT | ok |
| windowfunctions | crate | MIT | ok |
| windows | crate | MIT OR Apache-2.0 | ok |
| windows-collections | crate | MIT OR Apache-2.0 | ok |
| windows-core | crate | MIT OR Apache-2.0 | ok |
| windows-future | crate | MIT OR Apache-2.0 | ok |
| windows-implement | crate | MIT OR Apache-2.0 | ok |
| windows-interface | crate | MIT OR Apache-2.0 | ok |
| windows-link | crate | MIT OR Apache-2.0 | ok |
| windows-numerics | crate | MIT OR Apache-2.0 | ok |
| windows-result | crate | MIT OR Apache-2.0 | ok |
| windows-strings | crate | MIT OR Apache-2.0 | ok |
| windows-sys | crate | MIT OR Apache-2.0 | ok |
| windows-threading | crate | MIT OR Apache-2.0 | ok |
| winnow | crate | MIT | ok |
| writeable | crate | Unicode-3.0 | ok |
| yoke | crate | Unicode-3.0 | ok |
| yoke-derive | crate | Unicode-3.0 | ok |
| zerocopy | crate | BSD-2-Clause OR Apache-2.0 OR MIT | ok |
| zerocopy-derive | crate | BSD-2-Clause OR Apache-2.0 OR MIT | ok |
| zerofrom | crate | Unicode-3.0 | ok |
| zerofrom-derive | crate | Unicode-3.0 | ok |
| zeroize | crate | Apache-2.0 OR MIT | ok |
| zerotrie | crate | Unicode-3.0 | ok |
| zerovec | crate | Unicode-3.0 | ok |
| zerovec-derive | crate | Unicode-3.0 | ok |
| zmij | crate | MIT | ok |
| @babel/code-frame | package | MIT | ok |
| @babel/compat-data | package | MIT | ok |
| @babel/core | package | MIT | ok |
| @babel/generator | package | MIT | ok |
| @babel/helper-compilation-targets | package | MIT | ok |
| @babel/helper-globals | package | MIT | ok |
| @babel/helper-module-imports | package | MIT | ok |
| @babel/helper-module-transforms | package | MIT | ok |
| @babel/helper-string-parser | package | MIT | ok |
| @babel/helper-validator-identifier | package | MIT | ok |
| @babel/helper-validator-option | package | MIT | ok |
| @babel/helpers | package | MIT | ok |
| @babel/parser | package | MIT | ok |
| @babel/template | package | MIT | ok |
| @babel/traverse | package | MIT | ok |
| @babel/types | package | MIT | ok |
| @jridgewell/gen-mapping | package | MIT | ok |
| @jridgewell/remapping | package | MIT | ok |
| @jridgewell/resolve-uri | package | MIT | ok |
| @jridgewell/sourcemap-codec | package | MIT | ok |
| @jridgewell/trace-mapping | package | MIT | ok |
| @next/env | package | MIT | ok |
| @swc/helpers | package | Apache-2.0 | ok |
| baseline-browser-mapping | package | Apache-2.0 | ok |
| browserslist | package | MIT | ok |
| caniuse-lite | package | CC-BY-4.0 | ok |
| client-only | package | MIT | ok |
| convert-source-map | package | MIT | ok |
| debug | package | MIT | ok |
| electron-to-chromium | package | ISC | ok |
| escalade | package | MIT | ok |
| gensync | package | MIT | ok |
| js-tokens | package | MIT | ok |
| jsesc | package | MIT | ok |
| json5 | package | MIT | ok |
| lru-cache | package | ISC | ok |
| ms | package | MIT | ok |
| nanoid | package | MIT | ok |
| next | package | MIT | ok |
| node-releases | package | MIT | ok |
| picocolors | package | ISC | ok |
| postcss | package | MIT | ok |
| react | package | MIT | ok |
| react-dom | package | MIT | ok |
| scheduler | package | MIT | ok |
| semver | package | ISC | ok |
| source-map-js | package | BSD-3-Clause | ok |
| styled-jsx | package | MIT | ok |
| tslib | package | 0BSD | ok |
| update-browserslist-db | package | MIT | ok |
| yallist | package | ISC | ok |
| Noto Sans Mono | font | OFL-1.1 | ok |
| Pixelify Sans | font | OFL-1.1 | ok |
| Press Start 2P | font | OFL-1.1 | ok |
| .env.example | release | Apache-2.0 | ok |
| LICENSE | release | Apache-2.0 | ok |
| NOTICE | release | Apache-2.0 | ok |
| README.md | release | Apache-2.0 | ok |
| lumingo.exe | release | Apache-2.0 | ok |

The attribution table below has one row per shipped item whose licence asks
for something the package's own metadata cannot carry: the notice is written
here, in this project's own words, and `scripts/gen-notices.mjs` reads it from
this table and renders it under the item's own line in `NOTICE`. An item whose
licence needs no notice gets no row, because the licence the build states is
the whole of the permission. A row here for an item the build does not ship
is a false claim, and a shipped item that needs a notice but has no row here
is a notice that was left out; `scripts/check-license-register.mjs` fails on
both.

| Item | Notice |
|---|---|
| caniuse-lite | caniuse-lite, browser support data used by the Next.js build, is licensed under CC BY 4.0 (https://creativecommons.org/licenses/by/4.0/). It is not part of the shipped application. |
| colored | Unmodified, used as a dependency of harper-core 0.68.0. Its source is at https://crates.io/crates/colored and the Mozilla Public License 2.0 text is at https://www.mozilla.org/MPL/2.0/ |
| cssparser | Unmodified, used as a dependency of harper-core 0.68.0. Its source is at https://crates.io/crates/cssparser and the Mozilla Public License 2.0 text is at https://www.mozilla.org/MPL/2.0/ |
| dtoa-short | Unmodified, used as a dependency of harper-core 0.68.0. Its source is at https://crates.io/crates/dtoa-short and the Mozilla Public License 2.0 text is at https://www.mozilla.org/MPL/2.0/ |
