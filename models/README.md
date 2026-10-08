# Models

`manifest.toml` lists the speech models Lumingo can install. It is the only
source of download locations. No model file is stored in this repository or in
the release zip: the learner's machine fetches each one from the upstream
location the manifest names, after showing the licence.

`manifest.schema.json` is the schema, checked in the tests of
`crates/model-manager` against the TOML converted to JSON. The Rust parser is
the authority and also checks what a schema cannot: unique ids, file paths that
stay inside the model folder, and whether an entry can be downloaded at all.

## Entry fields

| Field | Meaning |
|---|---|
| `id` | Unique, `a-z 0-9 - _`. Names the install folder. |
| `role` | `vad`, `stt`, `tts` or `pron`. |
| `engine` | The runtime that loads the files, for example `sherpa-onnx` or `ort`. |
| `version` | The upstream version or export. `candidate` until it is chosen. |
| `license`, `license_url` | What the licence is and where it is read. `unknown` or an empty URL keeps the entry from downloading. |
| `license_text` | Optional. The licence text or use restrictions in full, shown before the download. |
| `license_review` | `true` when the licence has use restrictions that must be read in full, for example OpenRAIL-M. |
| `source` | Upstream location, never a project-owned mirror. A directory URL: a file without its own `url` is fetched from `source` plus `path`. |
| `size_bytes` | Total size, `0` until measured from a real download. |
| `files` | `{ path, sha256, url?, size_bytes? }`. `path` is relative to the model folder and uses `/`. |
| `notes` | What is still unverified. |

## Download rules

- An entry with no files, or any file with an empty or malformed `sha256`, is
  refused before any request. `model-manager verify` lists those entries.
- A download needs the licence notice to have been shown and accepted for the
  same licence the entry carries now.
- Every URL and every redirect must be `https`. Plain `http` is accepted for the
  loopback address only. URLs with credentials are refused.
- A download is written to `<file>.part`, resumed with `Range: bytes=N-`, and
  moved into place only when its SHA-256 matches. A mismatch keeps nothing.
- What is installed is recorded in `installed.json` in the models folder.

## Filling in a candidate

Every entry in `manifest.toml` is a candidate and none can be downloaded yet.
To make one downloadable, the owner downloads each file from the upstream host,
checks the licence at the source, computes the SHA-256 of the file they
downloaded, and fills in `files`, `size_bytes`, `version`, `license_url` and
`license_text`. A checksum copied from anywhere else does not count.
`docs/LICENSE_REGISTER.md` is updated in the same change.
