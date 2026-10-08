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
| Sessions | `session::SessionService` (the boundary), `session::SessionManager` (the implementation), `attach_sessions` | At most one active session. Without a service, `NotAvailable(Sessions)`. See "Sessions". |
| Speech models | `models`, `download_model`, `cancel_download` | Over `model-manager` and `models/manifest.toml`. See "Models". |
| Inspector | `inspector` | The last provider requests and responses, from `llm_client::PayloadLog`. |
| Engines | `engines::Engines`, `engines::file` | Which audio devices and speech engines this run has, loaded behind features. |
| Voice loop | `voice::VoiceLoop`, `VoiceHandle`, `VoiceEvent` | Listen, transcribe, think, speak over trait objects. Used by `tools/tutor-cli` and, through `SessionManager`, by the server. See below. |

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
| `GET /api/inspector` | `inspector` |
| `POST /api/sessions` | `start_session` |
| `POST /api/sessions/{id}/stop` (body optional) | `stop_session` |
| `POST /api/sessions/{id}/pause`, `POST /api/sessions/{id}/resume` | `pause_session`, `resume_session` |
| `POST /api/sessions/{id}/text` | `send_text` |
| `POST /api/sessions/{id}/turns/{seq}/edit` | `edit_turn` |
| `POST /api/sessions/{id}/push-to-talk` | `push_to_talk` |
| `POST /api/tutor/stop-speaking` | `stop_speaking` |
| `GET /api/sessions/{id}/next-activity`, `POST /api/activities/submit` | `next_activity`, `submit_activity` |
| `POST /api/writing/{session}/drafts` | `submit_draft` |
| `POST /api/reading/generate`, `POST /api/reading/{session}/answers` | `generate_reading`, `answer_reading` |
| `POST /api/tts/speak` | `speak` |
| `GET /api/audio/devices`, `POST /api/audio/test` | `audio_devices`, `audio_test` |
| `GET /api/models`, `POST /api/models/{id}/download`, `POST /api/models/{id}/cancel` | `models`, `download_model`, `cancel_download` |

`DELETE /api/sessions/{id}` deletes a stored session (data); `POST /api/sessions/{id}/stop` ends the running one. `{seq}` of the edit route is the turn number the events of that turn carry, not the stored position.

There is no route for `record_practice`: XP comes from finished practice inside the core, never from a request.

## Types

Every request, response and event type is in `api` and derives `ts_rs::TS`. `cargo test -p app-core` writes the declarations to `apps/web/src/generated/` (see `.cargo/config.toml`). CI regenerates them and fails on a diff. Enums of `storage` and `game` that cross the API are mirrored by the `api_enum!` macro, which writes both conversions as exhaustive matches.

## Sessions

`SessionManager::attach(core, engines)` creates the manager and attaches it to the core. The server calls it at start-up with what `Engines::detect` found.

| Kind | Runs on | Needs |
|---|---|---|
| `text_chat` | `tutor_engine::TextChat` | a provider |
| `conversation` | `voice::VoiceLoop` | audio devices and the VAD and recogniser; the synthesiser unless `speak` is off; a provider |
| `lesson`, `checkpoint`, `drill` | `tutor_engine::UnitPlayer` | the unit; a provider only for the roleplay and the productive activities, which wait without one |
| `writing` | `tutor_engine::Workshop` | a provider for the second and third layer; layer one works without |
| `reading` | `tutor_engine::ReadingSession` | a provider to generate a text; the authored sets are the fallback |
| `review` | none | `NotAvailable`: the review queue has a scheduler and nothing that presents its items |
| `placement` | none | `NotAvailable`: the placement item bank does not exist |

Rules:

- **One session at a time.** A start while one runs, or is starting, is refused with `Conflict` (HTTP 409). Nothing is queued. The slot goes through `Idle`, `Starting`, `Active`, `Ending`, so two starts at the same moment give one session and one refusal.
- **One turn at a time inside a session.** A second message while a reply runs is refused with `Busy`.
- `POST .../text` answers when the turn is accepted; the reply arrives as events. The session lives in the server, not in the page: a closed or reloaded page does not end it, and the snapshot carries `active_session` (the newest 12 lines, each cut at 1000 characters, and the partial reply, cut at 4000) so the page can rebuild the conversation.
- A lesson, checkpoint or drill lists the activities that cannot be done with what this program has (a pronunciation drill that needs a phoneme model and a recording, a spoken answer, an item played aloud without speech output) in `NextActivity.unavailable`, with the reason. It never scores what it could not measure.
- `tts/speak` is by reference: a tutor turn or a generated reading text of a stored session, or the audio lines of an activity of the running unit. The request has no field for text; a `text` field that is sent anyway is ignored. A refusal because the speech output is busy does not count as a play of a listening item.
- Free-mode and generated attempts are stored as free-mode work and never count toward an estimate. No model output sets or states a level; the level of a free mode is the learner's pick.
- `AnalysisReady` of a turn is held until that turn's reply is complete, so it never comes before it.
- Speech output outside a conversation (`tts/speak`) is a `VoiceLoop` without a microphone, started on the first request. A conversation, the audio test and the end of the program close it, because the devices belong to them.

### Events

All events carry a sequence number. `Snapshot`, `Heartbeat`, `ProviderStatus`, `SessionState`, `TurnState`, `MicLevel`, `TranscriptFinal`, `TutorTextDelta`, `TutorSentenceSpoken`, `PronFindings`, `AnalysisReady`, `FeedbackReady`, `LatencyReport`, `EngineStatus`, `DownloadProgress`, `Error`. The order inside a text turn is in the module documentation of `api/events.rs`: `TranscriptFinal`, `TurnState` thinking, `TurnState` replying with one `TutorTextDelta` per piece, `TurnState` waiting with the stored position of the reply, then `AnalysisReady`. The first event of a session is its `SessionState` `active`; engines that load for it report through `EngineStatus` before that. A stop sends `FeedbackReady` (the summary) and then `SessionState` `ended`. A spoken turn sends its `TranscriptFinal` twice, once when heard (no `turn_seq`) and once when stored.

`MicLevel` is at most ten a second (one tick every 100 ms, the first one a period after the start), and not sent while the level is silent.

## Models

`models` lists the entries of `models/manifest.toml` with their licences, whether each can be downloaded and why not, and what is installed. `download_model` starts a download only when the request names the licence that was shown (`accept_licence` and the same `license` and `license_url`); otherwise `LicenceNotAccepted` (HTTP 409). An entry with no files or an empty checksum is refused before any request. One download runs at a time (`Busy`); progress is `DownloadProgress`, at most ten a second per download plus the end of each file; `cancel_download` reaches the transfer within one read and keeps the partial file so the next download resumes. Every entry of the shipped manifest is a candidate with empty checksums, so none can be downloaded yet; the tests use a manifest and a server on the loopback address.

## Inspector

`GET /api/inspector` shows what left the machine, so the learner can check it. `llm_client::PayloadLog` is a ring of the last **50** requests and their responses; a body longer than **16 KiB** is cut and flagged. When the ring is full the oldest entry is dropped and counted (`dropped`). Headers are not kept at all, the key is removed from the text when an entry is captured (not when it is read), and a request that never finished is shown as `pending` or `abandoned`. It holds the learner's words, because they are in the requests, and it is not part of the export. See `crates/llm-client/README.md`.

## Engines

`Engines::detect` builds what the cargo features and the files on disk allow: `cpal-backend` (microphone and speakers) and `sherpa` (VAD, recognition and synthesis), both off by default, and `engines.toml` in the data folder (or `--engines-file`) naming the model files. With every feature off there is no audio and no speech, the snapshot says why for each engine, and text chat, lessons, writing and reading work. Routes that need them answer `not_available` (HTTP 501) naming what is missing. Fakes exist only in `engines::testing` and `voice::testing`, behind `test-support`.

## Channels

Two channels carry events to clients.

- The event bus, a Tokio broadcast channel of **capacity 64**, described below.
- The voice loop's own channels, each bounded, listed with what happens when full in `src/voice/mod.rs`. The server turns what comes out of them into bus events; a session task that is slow to publish is not waiting for any client.

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

Review sessions and the placement test (no presenter for the review queue, no item bank), pronunciation-scored drills and guided speaking (no recorder for a single clip and no phoneme model are wired), and a real microphone, speakers or speech model (see UNVERIFIED). `unavailable` in the snapshot is computed from what loaded: `speech` is there unless an audio backend and the speech engines are configured, `models` unless the manifest could be read, `activities` unless a unit loaded.

## UNVERIFIED

Nothing below has been run in this repository's build container.

- Real audio through cpal (`cpal-backend`): the container has no ALSA headers, and the Windows target could not link here.
- Real VAD, recognition and synthesis through sherpa-onnx (`sherpa`): `cargo check` with the library folder empty only.
- A voice conversation with a real microphone and a real model; the level meter, the endpointing and the latency numbers on real hardware.
- A real provider. Every test uses a scripted client or a small server on the loopback address.
- Downloading a real model: the manifest has no checksums yet.
- Windows, and the web UI against these routes (the generated types are checked by `pnpm typecheck`).

## Tests

`cargo test -p app-core` runs unit tests and integration tests against a real SQLite file in a temporary directory, with a fixed clock and a small OpenAI-compatible server on 127.0.0.1 for the provider tests. No test contacts a real provider. Nothing in this crate needs a microphone, a speaker or a model. The session tests (`tests/sessions_*.rs`, `models.rs`, `key_and_inspector.rs`, `engines.rs`) run the manager over a scripted language model, fake devices and fake speech engines, and the model tests over a server on the loopback address.

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
