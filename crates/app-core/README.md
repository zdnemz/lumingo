# app-core

The core of Lumingo without HTTP. `apps/server` parses a request, checks it, calls one method of `AppCore` and serialises the answer. Everything the server can do is also reachable from a test.

## What it owns

| Area | Where | Notes |
|---|---|---|
| Lifecycle | `AppCore::open`, `request_shutdown`, `close` | Opens and migrates the database, makes sure one learner profile exists, reads the provider profiles, indexes the units, measures the machine. |
| Settings | `settings`, `update_settings` | Profile fields on the profile row, two switches in `app_settings`. |
| Hardware profile | `hardware` | Installed memory and logical processors as the OS reports them; `None` when unknown. The floor is 8 GB (decimal) and 4 logical processors. |
| Events | `events::EventBus`, `AppCore::snapshot_event` | See "Channels". |
| Providers | `list_providers`, `save_provider`, `delete_provider`, `activate_provider`, `test_provider`, `llm_client` | Over `llm_client::ProfileSet`. No function returns a key. |
| Curriculum | `list_units`, `unit`, `reindex_curriculum` | Loads a folder through `curriculum`, writes the index to storage. |
| Progress | `progress`, `attempt_evidence` | Read models over storage. Nothing is computed here. |
| Game | `game_state`, `equip`, `record_practice`, `grant_rest_token` | Cosmetic. Reads and writes no attempt, evidence or estimate. |
| Data | `delete_session`, `delete_all_data`, `export` | Recordings are deleted before rows; the file is compacted afterwards. |
| Diagnostics | `diagnostics` | Hardware, latency percentiles from stored samples, provider test result. |
| Sessions | `session::SessionService`, `attach_sessions`, `session_service` | A boundary only. Without a service, `NotAvailable(Sessions)`. |
| Voice loop | `voice::VoiceLoop`, `VoiceHandle`, `VoiceEvent` | Listen, transcribe, think, speak over trait objects. Used by `tools/tutor-cli` now and by the server through `SessionService` later. Not attached to the server yet. See below. |

## Route to method

| Route | Method |
|---|---|
| `GET /api/state` | `snapshot` |
| `GET /api/units`, `GET /api/units/{id}` | `list_units`, `unit` |
| `GET/POST /api/providers`, `DELETE /api/providers/{id}` | `list_providers`, `save_provider`, `delete_provider` |
| `POST /api/providers/{id}/test`, `POST /api/providers/{id}/activate` | `test_provider`, `activate_provider` |
| `GET /api/progress`, `GET /api/attempts/{id}/evidence` | `progress`, `attempt_evidence` |
| `GET /api/game`, `POST /api/game/equip` | `game_state`, `equip` |
| `GET/PUT /api/settings` | `settings`, `update_settings` |
| `DELETE /api/sessions/{id}`, `DELETE /api/data`, `GET /api/export` | `delete_session`, `delete_all_data`, `export` |
| `GET /api/diagnostics` | `diagnostics` |

There is no route for `record_practice`: XP comes from finished practice inside the core, never from a request.

## Types

Every request, response and event type is in `api` and derives `ts_rs::TS`. `cargo test -p app-core` writes the declarations to `apps/web/src/generated/` (see `.cargo/config.toml`). CI regenerates them and fails on a diff. Enums of `storage` and `game` that cross the API are mirrored by the `api_enum!` macro, which writes both conversions as exhaustive matches.

## Channels

There is one channel: the event bus, a Tokio broadcast channel of **capacity 64**.

- Publishing never waits and never fails because nobody listens.
- When a subscriber is more than 64 events behind, the channel drops the oldest events for that subscriber only. Its next receive reports `Lagged`, and the transport (`apps/server/src/ws.rs`) sends it a fresh `Snapshot` instead of the missed events. Events with a sequence number not above the snapshot's are skipped.
- Sequence numbers are assigned and sent under one lock, so subscribers see them in order. A snapshot is built under the same lock.

Other bounded things:

- One provider connection test at a time. A second request is refused with `Busy` (HTTP 409), not queued.
- Request bodies are at most 64 KiB (`apps/server`); larger ones get HTTP 413.
- Review items in the progress overview: at most 50, with a `reviews_due_truncated` flag. Recent sessions: at most 50.

## Rules kept here

- A key is held in memory by `llm-client` and, for saved profiles, in `providers.toml` in plain text. It is never written to SQLite, logs, the export or an event, and no answer contains it. The UI gets `has_key` and the last four characters. The export has neither.
- A `providers.toml` that cannot be read is not overwritten; saving is refused until it is fixed.
- Blocking work (file reads and writes, schema checks, memory probes) runs through `spawn_blocking`. Database calls are async.
- A provider test is cancelled when the program shuts down or when the request future is dropped.
- The game layer is cosmetic. XP amounts depend on the kind of practice and not on a score. Free-mode practice may earn XP and never reaches an estimate.
- The core grants no rest-day token by itself: the grant policy is an open product question. `grant_rest_token` exists for whoever decides it.

## Not here

Lessons, activities, free modes and model downloads, and the server's use of the voice loop (it is not attached to `SessionService`). The snapshot lists them in `unavailable`. The payload inspector (`GET /api/inspector`) needs a hook in `llm-client` and is not built.

## Tests

`cargo test -p app-core` runs unit tests and integration tests against a real SQLite file in a temporary directory, with a fixed clock and a small OpenAI-compatible server on 127.0.0.1 for the provider tests. No test contacts a real provider. Nothing in this crate needs a microphone, a speaker or a model.

## The voice loop (`voice`, ROADMAP S3-06)

`voice::VoiceLoop` runs the conversation by voice: microphone, gate, VAD and
endpointer, STT worker, the T1 prompt, the LLM stream, the sentence chunker, TTS worker
and playback. It is wired over trait objects only (`DeviceRegistry` over an
`AudioBackend`, `Vad`, `SttEngine`, `TtsEngine`, `Arc<dyn LlmClient>`) and reuses the
tutor engine's `Session` as its state machine. The module documentation in
`src/voice/mod.rs` lists the threads, the capacity of every queue and what happens when
it is full, and how a stop or a barge-in cancels the model stream, the TTS turn and
playback.

* Every turn records the five latency parts of `context_pack.md` section 4 from an
  injected `LoopClock`, and `LatencySummary` gives p50 and p95.
* A failed provider call is retried once; the second failure moves the session to
  `ProviderUnavailable` with a message and waits for `resume`. `VoiceConfig::provider_timeout`
  is the whole budget for both attempts.
* The background analysis (T2) is spawned through the tutor engine's `TurnAnalyzer` by a
  recorder task that is not on the speech path. Storing is optional (`Recording`).
* `Scenario::from_unit` builds the prompt context from a unit's roleplay.
* The fakes (`voice::testing`: manual clock, scripted language model, fake engines, fake
  audio backend with a held output) exist only for this crate's tests and behind the
  `test-support` feature. A release build does not contain them.

`cargo test -p app-core` runs the loop over the fakes in `tests/voice_loop.rs`,
`tests/voice_record.rs` and `tests/voice_offline.rs` (a closed port, a listener that
never answers, an address that routes nowhere, a capture server that proves no request
goes to any host but the provider). No test needs a microphone, a speaker, a model or the
network. A real conversation is UNVERIFIED: see `tools/tutor-cli/README.md`.
