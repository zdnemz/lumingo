import { afterEach, test, type Browser } from "@e2e-dev/web";
import { expect, type Locator, type Screen } from "e2e";
import { readFileSync, readdirSync } from "node:fs";
import { join, resolve } from "node:path";

/**
 * The settings screens, end to end, against the offline server
 * `pnpm test:e2e` starts by itself.
 *
 * The spec reaches `/settings/` through the menu, which is what proves the nav
 * entry end to end, and then walks the three settings pages in the order a
 * learner meets them: the general page (the provider panel, display and
 * language, the learner's own settings, the timing and recordings controls, the
 * honest audio-devices note and the model manager), the diagnostics page, and
 * the privacy page (the statements, the data locations, the payload inspector,
 * the export and both deletes).
 *
 * Everything the spec asserts about the model manager is read from
 * `models/manifest.toml`, the file `scripts/e2e-server.sh` hands the server with
 * `--models-manifest`. The ids, versions, licences, licence links and the
 * reason each entry cannot be downloaded are derived from that file, so the
 * spec cannot disagree with the manifest the run really serves. Every entry is a
 * candidate with `files = []`, which is the honest offline state of the model
 * manager: the panel shows the server's own reason and offers no download
 * control, because a real download needs checksums only the owner can fill in.
 *
 * One provider profile is typed in through the form — the pinned fake of the
 * wave-2 contract, whose base URL is a closed loopback port — so the card, the
 * export and the diagnostics page can be proved against a real saved profile
 * while nothing ever leaves the machine. The spec never registers the key as a
 * `Secret`: it asserts the key and its middle appear nowhere on the page and
 * nowhere in the exported bytes, which is only a meaningful assertion while the
 * runner is not rewriting the file it was served.
 *
 * **The profile is deleted before this spec ends, and again in `afterEach`.**
 * That is a hard rule, not a courtesy: `tests/talk.e2e.ts` asserts the server's
 * own `provider_not_configured` refusal, which only exists while no provider is
 * saved, and the suite shares one server.
 *
 * Two orderings carry a proof rather than a preference:
 *
 * - The diagnostics page is read **before** the writing session is created. A
 *   draft's analysis is attempted even when it must fail, and `CallLog` writes
 *   its row before the reply is checked, so a session made first would put
 *   provider calls on the counter and the "no call was made" clause could not be
 *   stated. Read first, the counter is compared with the baseline taken at the
 *   start of this walk: the difference is what this spec caused, and it is zero.
 * - The payload inspector is empty for the same reason: offline nothing can be
 *   sent, and saving a profile is not a request. The populated case is proved in
 *   Rust by `s6-10-key`.
 *
 * Interactions use `screen.tapAt` at a point inside the node rather than
 * `locator.tap`. Both are the same learner gesture, but a locator tap first
 * waits for the node to hold still across animation frames, and this headless
 * browser stops producing frames once the page has settled and nothing is
 * painted — so that wait never ends. A point tap is dispatched straight at the
 * point the node reports, which is what a person's finger does. Typing uses
 * `fill`, which needs no frames. The page is laid out in a viewport tall enough
 * to hold the whole settings page — the general page carries ten model rows —
 * so a point is always on screen and nothing has to be scrolled.
 */

/** A viewport tall enough that the whole screen fits: no scrolling, so a point tap always lands. */
const TALL_VIEWPORT = { width: 1280, height: 16000 };

/**
 * The total budget for one `shellSettled` probe, the first slice of it one
 * attempt takes, the ceiling that slice grows to, and the pause between attempts.
 *
 * The budget is well above the 15 s a single wait was given, because what it
 * buys is another *read*, not a longer wait: see the helper for why that
 * distinction is the whole fix. The budget is measured, not guessed: the longest
 * stall seen at one call site — the navigation right after the export download —
 * ran four consecutive dead reads, 32 s of no answer at all, and then settled. A
 * 30 s budget gave up in the middle of exactly that stall and failed the run, so
 * the budget has to clear it. Thirty-five seconds does, with margin, and is also
 * as large as this can safely go: the stall costs the budget in full, and the
 * whole spec still has to finish inside its own attempt deadline.
 *
 * The attempt grows because the two ways a read fails here pull in opposite
 * directions, and no single size serves both.
 *
 * - A read that lands in the document swap hangs and burns its **whole** timeout
 *   without answering — measured at 6001 ms of a 6 s attempt, twice in a row, in
 *   a run that then settled on the next read. Only a *fresh* read escapes it, so
 *   a short first attempt notices the swap ending soonest.
 * - A read on a loaded machine is merely slow and needs an attempt long enough to
 *   finish. The longest honest wait seen here is 5.3 s, so a probe that only ever
 *   tried 2 s would cut that read off forever and never settle at all.
 *
 * So the slice starts short, to get past a swap quickly, and doubles after each
 * failure up to the ceiling, so a genuinely slow read still gets an attempt it
 * can finish in. The pause is a cadence between reads, not a guess at how long a
 * swap takes.
 */
const SHELL_PROBE_BUDGET_MS = 35_000;
const SHELL_PROBE_FIRST_ATTEMPT_MS = 2_000;
const SHELL_PROBE_ATTEMPT_CEILING_MS = 10_000;
const SHELL_PROBE_PAUSE_MS = 150;

/** The manifest `scripts/e2e-server.sh` serves, relative to the project root. */
const MANIFEST_FILE = "models/manifest.toml";

/** The pinned fake provider of the wave-2 contract. Its address is a closed loopback port. */
const PROVIDER = {
  name: "e2e-temporary",
  protocol: "openai_chat",
  baseUrl: "http://127.0.0.1:9/v1",
  model: "e2e-model",
  key: "sk-e2e-SECRETMATERIAL-2f6e5d4c",
  /** The middle of the key: the part no screen and no export may ever show. */
  middle: "SECRETMATERIAL",
  /** What the card may show: the last four characters of the key, and nothing more. */
  last4: "sk-e2e-SECRETMATERIAL-2f6e5d4c".slice(-4),
} as const;

/** The writing session the spec creates only in order to delete it. */
const PROMPT = "Write about my city.";
const DRAFT = "This is an test. I am from Surabaya and I live near the market.";

/** One entry of `models/manifest.toml`, as far as this spec reads it. */
interface ManifestEntry {
  id: string;
  version: string;
  license: string;
  licenseUrl: string;
  /** The raw `files` value. Every candidate in the shipped manifest has `[]`. */
  files: string;
}

/**
 * Reads the manifest the offline server was pointed at. Only the fields the
 * panel draws are kept: the id, the version, the licence, its link, and the
 * `files` list the downloadability rule turns on. A small line reader is enough
 * for the shape this file has, and it keeps the spec free of a TOML dependency
 * the project does not otherwise carry.
 */
function readManifest(): ManifestEntry[] {
  const text = readFileSync(resolve(process.cwd(), MANIFEST_FILE), "utf8");
  const assignment = /^([a-z_]+)\s*=\s*(.+)$/;
  const entries: ManifestEntry[] = [];
  let current: ManifestEntry | null = null;
  const finish = () => {
    if (current === null) return;
    if (current.id === "") throw new Error(`${MANIFEST_FILE} has a model entry with no id`);
    entries.push(current);
    current = null;
  };
  for (const rawLine of text.split("\n")) {
    const line = rawLine.trim();
    if (line === "" || line.startsWith("#")) continue;
    if (line === "[[model]]") {
      finish();
      current = { id: "", version: "", license: "", licenseUrl: "", files: "" };
      continue;
    }
    if (current === null) continue;
    const match = assignment.exec(line);
    if (match === null) continue;
    const key = match[1] ?? "";
    const raw = (match[2] ?? "").trim();
    const value = raw.length >= 2 && raw.startsWith('"') && raw.endsWith('"') ? raw.slice(1, -1) : raw;
    if (key === "id") current.id = value;
    else if (key === "version") current.version = value;
    else if (key === "license") current.license = value;
    else if (key === "license_url") current.licenseUrl = value;
    else if (key === "files") current.files = value;
  }
  finish();
  if (entries.length === 0) throw new Error(`${MANIFEST_FILE} lists no model entries`);
  return entries;
}

const MANIFEST = readManifest();

/**
 * Why the server says an entry cannot be downloaded. `ModelEntry::check_downloadable`
 * refuses a licence it has not read before it looks at the files, so an entry
 * with no usable licence reports that, and every other candidate reports that it
 * lists no files. The sentences are the server's own, worded here from the
 * manifest rather than copied from a previous run.
 */
function cannotBeDownloaded(entry: ManifestEntry): string {
  const license = entry.license.trim();
  if (license === "" || license.toLowerCase() === "unknown" || entry.licenseUrl.trim() === "") {
    return `model ${entry.id} has no license or no license_url`;
  }
  return `model ${entry.id} lists no files`;
}

/** The screen's own strings, from `apps/web/src/i18n/parts/{settings,shell,progress,write}.en.ts`. */
const TEXT = {
  /** `nav.settings`, `nav.progress` and `nav.write`: the menu entries this spec walks through. */
  navSettings: "Settings",
  navProgress: "Progress",
  navWrite: "Write",
  /** `settings.title`, the page's own heading. */
  title: "Settings",
  /** `settings.nav`: the accessible name of the settings sub-navigation. */
  navLabel: "Settings sections",
  /** `settings.nav.general`, `settings.nav.privacy` and `settings.nav.diagnostics`: the three links. */
  navGeneral: "General",
  navPrivacy: "Privacy and data",
  navDiagnostics: "Diagnostics",
  /** `settings.providers.title`, the panel the profiles live in. */
  providersTitle: "AI provider",
  /** `provider.list.empty`, the honest state of a fresh data directory. */
  providerEmpty: "No provider profiles yet.",
  /** `settings.providers.add`, the control that opens the form. */
  providersAdd: "Add a profile",
  /** `settings.display`, the panel that holds the display, theme and language controls. */
  display: "Display",
  /** `settings.language` and `settings.language.en`. */
  language: "Language",
  languageEn: "English",
  /** `settings.name`, the label of the learner's name field. */
  name: "What the tutor calls you",
  /** `settings.l1help`, the legend of the Indonesian-help radio group. */
  l1help: "Help in Indonesian",
  /** `settings.timing.title`, the legend of the timing radio group and its panel title. */
  timingTitle: "Pronunciation timing",
  /** `settings.timing.auto`, `settings.timing.wait` and `settings.timing.never`. */
  timingAuto: "Auto",
  timingWait: "Always wait",
  timingNever: "Never wait",
  /** `settings.timing.effect`: the panel says the choice changes nothing yet. */
  timingEffect:
    "Speech is not part of this build yet, so the choice changes nothing for now. It is saved and will be used once speech is here.",
  /** `settings.recordings.title`, the switch's accessible name. */
  recordingsTitle: "Recordings",
  /** `settings.recordings.hint`, the sentence under the switch. */
  recordingsHint:
    "Keep recent recordings of your voice so you can replay them. Speech is not part of this build yet, so nothing is recorded for now.",
  /** `privacy.audio`, which the switch must sit next to. */
  privacyAudio: "Raw audio is not kept unless you switch on recordings in Settings.",
  /** `settings.audio.title`, the panel the speech note lives in. */
  audioTitle: "Audio devices",
  /** `unavailable.title` and `unavailable.body`, with `feature.speech`. */
  unavailableTitle: "Not available in this build",
  speechMissing:
    "Speech: microphone, speakers and voices: not part of this version of the program yet, so there is no control for it here.",
  /** `settings.models.title`, the model manager's panel. */
  modelsTitle: "Speech models",
  /** `settings.models.empty`, which must not appear: the shipped manifest has entries. */
  modelsEmpty: "The model list is empty.",
  /** `settings.models.version`, `settings.models.licence`, `settings.models.licence.open` and `settings.models.why`. */
  modelsVersion: (version: string) => `Version ${version}`,
  modelsLicence: (license: string) => `Licence: ${license}`,
  licenceOpen: "Read the licence",
  modelsWhy: (reason: string) => `Why it cannot be downloaded: ${reason}`,
  /** `settings.models.download`, which no row may offer: every entry is a candidate. */
  modelsDownload: "Download this model",
  /** `provider.name`, `provider.protocol`, `provider.base_url`, `provider.model` and `provider.key`: the form's labels. */
  providerName: "Profile name",
  providerProtocol: "Protocol",
  providerProtocolOpenai: "OpenAI-compatible chat",
  providerBaseUrl: "Base URL",
  providerModel: "Model name",
  providerKey: "API key",
  /** `provider.save`, the form's submit control. */
  providerSave: "Save profile",
  /** `provider.key.has`, with the last four characters: all of the key the page may show. */
  keySaved: (last4: string) => `Key saved. It ends with ••••${last4}.`,
  /** `provider.tested.never`, the honest state of a profile no probe has passed. */
  testedNever: "Not tested yet.",
  /** `provider.delete` and `provider.delete.confirm.yes`. */
  providerDelete: "Delete",
  providerDeleteYes: "Yes, delete it",
  /** `data.title`, the privacy page's heading. */
  dataTitle: "Privacy and data",
  /** The five statements PRD 12.4 wants asserted. */
  privacyVoice:
    "Your voice stays on your device. Audio is handled by the program on this computer. It never reaches the browser or the network.",
  privacyText:
    "Text is sent to the AI provider you choose. That means what you type, what is written down from your speech, your writing drafts, and the lesson context inside the prompts: your level, the unit, error categories and rubric text.",
  privacyKey:
    "Your API key is saved in plain text in a file on this computer (providers.toml in the program's data folder). Programs that run as the same Windows user can read that file. A key from a .env file stays where you put it.",
  privacyLocal:
    "Runs on your own computer and opens in your browser. The program listens only on this computer and refuses requests from other pages.",
  /** `data.where.data`, `data.where.units` and `data.where.address`: the facts the locations panel draws. */
  whereData: "Data folder",
  whereUnits: "Lesson folder",
  whereAddress: "Address of the program",
  /** `data.inspector.title` and `data.inspector.empty`. */
  inspectorTitle: "Payload inspector",
  inspectorEmpty: "Nothing has been sent to a provider yet. The first request the tutor makes appears here.",
  /** `data.inspector.limits` and `data.inspector.dropped`, worded with the server's own numbers. */
  inspectorLimits: (entries: number, bytes: number) =>
    `Keeps the newest ${entries} entries, and cuts each request or response body at ${bytes} bytes.`,
  inspectorDropped: (count: number) => `Older entries are removed when the list is full. Dropped so far: ${count}.`,
  /** `data.export.button` and `data.export.done`. */
  exportButton: "Download my data",
  exportDone: (filename: string) => `The file ${filename} was handed to your browser. Your browser decides where it goes.`,
  /** `data.session.delete`, `data.session.confirm.phrase` and `data.session.done`. */
  sessionDelete: (id: number) => `Delete session ${id}`,
  sessionConfirm: (id: number) => `Type the session number ${id} to confirm.`,
  sessionDone: (id: number) => `Session ${id} was deleted.`,
  /** `data.session.kind.writing`, the kind the created session's row names. */
  sessionKindWriting: "writing",
  /** `data.deleted.audio`, the outcome line both deletes draw. */
  audioRemoved: (count: number) => `Recordings removed: ${count}.`,
  /** `data.all.button`, `data.all.confirm.phrase` and `data.all.done`. */
  allButton: "Delete everything",
  allConfirm: (phrase: string) => `Type ${phrase} to confirm.`,
  allDone: "Everything was deleted.",
  /** `data.confirm.phrase.all`, the phrase the learner must type. */
  allPhrase: "delete everything",
  /** `data.confirm.label` and `data.confirm.yes`, the typed confirmation's field and button. */
  confirmLabel: "Confirmation",
  confirmYes: "Delete",
  /** `progress.empty.title`, the state the progress page is back to after the wipe. */
  progressEmpty: "Nothing is recorded yet",
  /** `progress.title`, the progress page's own heading. */
  progressTitle: "Progress",
  /** `write.title`, `write.prompt.label`, `write.start`, `write.draft.label`, `write.check` and `write.finish`. */
  writeTitle: "Writing workshop",
  writePrompt: "Your task",
  writeStart: "Start writing",
  writeDraft: "Your draft",
  writeCheck: "Check this draft",
  writeFinish: "Finish the session",
  /** `write.ended.title`, the panel a finished session shows. */
  writeEnded: "Session finished",
  /** `write.rules.title`, which renders only once the server took the draft. */
  writeRules: "What the rule checker found",
  /** `diag.title`, the diagnostics page's heading. */
  diagTitle: "Diagnostics",
  /** `diag.program.title` and its five facts. */
  diagProgram: "Program",
  diagVersion: "Version",
  diagMode: "Mode",
  diagUptime: "Running for",
  diagAddress: "Address",
  diagSchema: "Database version",
  /** `link.mode.normal`: the offline server is not in development mode. */
  modeNormal: "Normal",
  /** `diag.hardware.title` and the three hardware facts. */
  diagHardware: "This computer",
  hardwareRam: "Memory",
  hardwareCores: "Logical processors",
  hardwareMinimum: "Minimum",
  /** `hardware.gb`, and the three verdict sentences. */
  gigabytes: (count: string) => `${count} GB`,
  hardwareMeets: (ram: string, cores: number) =>
    `This computer meets the minimum of ${ram} memory and ${cores} logical processors.`,
  hardwareBelow: (ram: string, cores: number) =>
    `This computer is below the minimum of ${ram} memory and ${cores} logical processors. The program may be slow.`,
  hardwareUnknown: (ram: string, cores: number) =>
    `The system did not report everything, so the program cannot tell whether this computer meets the minimum of ${ram} memory and ${cores} logical processors.`,
  /** `diag.provider.title` and `diag.provider.untested`. */
  diagProvider: "Active AI provider",
  diagProviderUntested: "The active provider has not passed a connection test yet.",
  /** `diag.curriculum.title` and `diag.curriculum.units`. */
  diagCurriculum: "Lessons",
  diagUnits: "Unit files found",
  /** `diag.latency.title`, `diag.latency.none` and `diag.latency.empty`. */
  diagLatency: "Speed of each step",
  latencyNone: "n/a",
  latencyEmpty: "No measurements yet. They appear after the first voice conversations.",
  /** `diag.llm.title`, `diag.llm.calls` and `diag.llm.empty`. */
  diagLlm: "Calls to the provider",
  diagLlmCalls: "Recent calls",
  diagLlmEmpty: "No provider calls recorded yet.",
};

/** The parts of the reports this spec reads back through the page's own routes. */
interface SettingsReport {
  adaptive_timing: string;
  keep_recordings: boolean;
}

interface ProviderReport {
  id: number;
  name: string;
  base_url: string;
  model: string;
  key_last4: string | null;
}

interface DiagnosticsReport {
  server_version: string;
  dev_mode: boolean;
  server_address: string | null;
  data_dir: string;
  curriculum_dir: string;
  schema_version: number;
  hardware: {
    ram_total_bytes: number | null;
    logical_cores: number | null;
    minimum_ram_bytes: number;
    minimum_logical_cores: number;
    meets_minimum: boolean | null;
  };
  provider: ProviderReport | null;
  curriculum: { unit_count: number; issue_count: number };
  latency: { metric: string; stats: { count: number; p50_ms: number | null; p95_ms: number | null } }[];
  llm: { calls: number; failures: number };
}

interface InspectorReport {
  capacity: number;
  max_body_bytes: number;
  dropped: number;
  entries: unknown[];
}

interface ProgressReport {
  recent_sessions: { id: number; kind: string }[];
}

/**
 * The middle of a node: the point a learner's finger lands on.
 *
 * The box is read again a moment later, and only a box that did not move is
 * tapped. The shell draws in stages — the header and the heading first, then the
 * banners once the server snapshot arrives, and a banner pushes everything below
 * it down by its own height — and every panel under it swaps its own "Loading..."
 * line for the data it fetched, which moves what is below it again. A point
 * measured during one of those stages is a point somewhere else by the time the
 * finger lands, which is how a tap aimed at a timing radio or a sub-navigation
 * link can miss it entirely.
 *
 * `locator.tap` cannot be used for this at all: it waits for the node to hold
 * still across animation frames, and this headless browser stops producing
 * frames once the page settles, so that wait never ends.
 */
async function tapAt(screen: Screen, locator: Locator): Promise<void> {
  let box = await locator.boundingBox();
  if (box === null) throw new Error("the node has no box to tap");
  for (let attempt = 0; attempt < 20; attempt += 1) {
    await new Promise((resolve) => setTimeout(resolve, 100));
    const again = await locator.boundingBox();
    if (again === null) throw new Error("the node has no box to tap");
    const steady =
      again.x === box.x && again.y === box.y && again.width === box.width && again.height === box.height;
    box = again;
    if (steady) break;
  }
  await screen.tapAt({ x: box.x + box.width / 2, y: box.y + box.height / 2 });
}

/**
 * Waits until the shell has finished drawing, which is when nothing above the
 * page's own content is still moving.
 *
 * Every page mounts its own `AppShell`, and the export ships nothing but the
 * boot screen, so a navigation replaces the document and the shell is drawn
 * after it: the header and the heading first, then the banners once the server
 * snapshot arrives, and a banner pushes everything below it down by its own
 * height. Waiting for that region is waiting for the last thing above the
 * content to stop moving, and it is what keeps the next measurement from being
 * taken against a page that is still arriving.
 *
 * `tests/a11y.e2e.ts` waits for `.shell__banners:not(:empty)` there, because
 * offline the no-provider banner is always drawn and is the shell's own signal.
 * This spec saves a provider profile half way through the walk, and from that
 * moment the shell honestly draws no banner at all: an empty region is then the
 * settled state, so waiting for content would never end. The wait is therefore
 * for the shell to be drawn and for the content under its banners to hold still,
 * which is the same fact the banner stood for and holds in both halves of the
 * walk.
 *
 * **Why this retries the whole probe instead of waiting longer.** A step that
 * taps a link and then asserts the URL is done as soon as the URL changes, and
 * that is before the new document has finished swapping in: the old document is
 * gone and the new one is not yet readable. A locator read aimed into that gap
 * does not fail fast — it hangs for its entire timeout and reports **zero
 * reads**, because not one query ever reached a document. That is not a node
 * that is slow to appear, so a longer `waitFor` timeout cannot help it: it only
 * gives the same hung read more time to hang, and raising it is the mistake that
 * made an earlier attempt at this look fixed. The probe is therefore *re-run*,
 * on a fresh read, until the shell answers: each attempt is bounded, a thrown or
 * timed-out read is caught and retried after a short pause, and only the total
 * budget ends the wait. The pause is a cadence between reads, not a guess at how
 * long a swap takes.
 *
 * The "content region holds still" measurement stays, because it is what makes
 * the helper right at the call sites that navigate and at the ones that do not:
 * being drawn is not the same as being finished drawing.
 */
async function shellSettled(browser: Browser): Promise<void> {
  const deadline = Date.now() + SHELL_PROBE_BUDGET_MS;
  let attemptMs = SHELL_PROBE_FIRST_ATTEMPT_MS;
  for (;;) {
    try {
      // Only that the shell is drawn: after the provider is saved this region is
      // legitimately empty, so its *content* is never what is waited for.
      await browser.locator(".shell__banners").waitFor({ state: "attached", timeout: attemptMs });
      const main = browser.locator(".shell__main");
      let box = await main.boundingBox();
      if (box === null) throw new Error("the shell has no content region to measure");
      for (let attempt = 0; attempt < 20; attempt += 1) {
        await new Promise((resolve) => setTimeout(resolve, 100));
        const again = await main.boundingBox();
        if (again === null) throw new Error("the shell's content region went away");
        const steady =
          again.x === box.x && again.y === box.y && again.width === box.width && again.height === box.height;
        box = again;
        if (steady) break;
      }
      return;
    } catch (cause) {
      // A read that landed in the document swap. Start the probe again from the
      // top, on a fresh read, for as long as the budget lasts — and give the next
      // one more room, in case this was a slow read rather than a dead one.
      if (Date.now() + SHELL_PROBE_PAUSE_MS >= deadline) throw cause;
      await new Promise((resolve) => setTimeout(resolve, SHELL_PROBE_PAUSE_MS));
      attemptMs = Math.min(attemptMs * 2, SHELL_PROBE_ATTEMPT_CEILING_MS);
    }
  }
}

/**
 * Reads a JSON route through the page's own origin and cookie, the way the
 * screen's client does. The text is parsed here rather than inside the page so
 * the value crossing the boundary is a string, which is always JSON-safe.
 */
async function apiGet<T>(browser: Browser, path: string): Promise<T> {
  const answer = (await browser.evaluate(async (url: string) => {
    const response = await fetch(url);
    return `${response.status} ${await response.text()}`;
  }, path)) as string;
  const space = answer.indexOf(" ");
  const status = answer.slice(0, space);
  const body = answer.slice(space + 1);
  if (status !== "200") throw new Error(`${path} answered ${status}: ${body}`);
  return JSON.parse(body) as T;
}

/**
 * Sends a state-changing request through the page's own origin and cookie. The
 * body is `{}` because the server refuses a state-changing request without a
 * JSON content type. Returns the status.
 */
async function apiSend(browser: Browser, method: "DELETE" | "POST", path: string): Promise<number> {
  return (await browser.evaluate(
    async (request: { method: string; path: string }) => {
      const response = await fetch(request.path, {
        method: request.method,
        headers: { "Content-Type": "application/json" },
        body: "{}",
      });
      return response.status;
    },
    { method, path },
  )) as number;
}

/**
 * The id of the session the server is holding, read through the page's own state
 * route. `null` means the server has no session, which is the state every spec
 * has to leave it in: the server runs one session at a time and refuses a second
 * start with `Conflict`.
 */
async function activeSessionId(browser: Browser): Promise<number | null> {
  return (await browser.evaluate(async () => {
    const state = await (await fetch("/api/state")).json();
    return state.active_session === null ? null : state.active_session.id;
  })) as number | null;
}

/** Ends a session through `POST /api/sessions/{id}/stop`, the route every screen's client calls for its finish control. */
async function stopSession(browser: Browser, id: number): Promise<number> {
  return (await browser.evaluate(async (sessionId: number) => {
    const response = await fetch(`/api/sessions/${sessionId}/stop`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: "{}",
    });
    return response.status;
  }, id)) as number;
}

/**
 * The file the runner saved for this download. `browser.waitForDownload`
 * reports the path relative to the attempt's artifact directory, so the file is
 * found by that suffix under the run's results tree rather than by rebuilding
 * the attempt directory's name here. The run empties its results before its
 * tests start, so a suffix match is this run's own download.
 */
function downloadOnDisk(reported: { path: string; suggestedFilename: string }): string {
  const root = resolve(process.cwd(), ".e2e", "results");
  const wanted = reported.path.replaceAll("\\", "/");
  const matches: string[] = [];
  const walk = (directory: string): void => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const full = join(directory, entry.name);
      if (entry.isDirectory()) walk(full);
      else if (full.replaceAll("\\", "/").endsWith(wanted)) matches.push(full);
    }
  };
  walk(root);
  const [only] = matches;
  if (only === undefined || matches.length !== 1) {
    throw new Error(`expected one file at ${wanted} under ${root}, found ${matches.length}`);
  }
  return only;
}

/** A byte count as the screen writes it: decimal units, one decimal at most. */
function gigabytes(bytes: number): string {
  const value = Math.round((bytes / 1_000_000_000) * 10) / 10;
  return Number.isInteger(value) ? String(value) : value.toFixed(1);
}

/**
 * One panel of the current page, found by the title in its own heading.
 *
 * The match is on the heading, never the panel's body: `filter({ hasText })`
 * reads the whole text content as a case-insensitive substring, so a sentence
 * in one panel's prose that happens to use another panel's title word would
 * match both. `Panel` renders its title in `h2.px-panel__title`, so scoping the
 * filter to that heading keeps a body sentence out of the match.
 */
function panel(browser: Browser, title: string): Locator {
  return browser.locator("section.px-panel").filter({
    has: browser.locator(".px-panel__title").filter({ hasText: title }),
  });
}

/**
 * Whatever this spec did, the next one must find the server in the state it
 * expects. Two belts:
 *
 * - no provider profile may be left saved, because `tests/talk.e2e.ts` asserts
 *   the server's own `provider_not_configured` refusal and that refusal only
 *   exists while none is saved. The walk deletes the profile through the screen;
 *   this repeats the delete through the route for a walk that failed half way.
 * - no session may be left running, because the server's single session slot is
 *   shared by the whole suite.
 *
 * A page that never reached the app has no origin to ask, and then there is
 * nothing this spec could have created either.
 */
afterEach(async ({ browser }) => {
  try {
    const running = await activeSessionId(browser);
    if (running !== null) await stopSession(browser, running);
  } catch {
    // The page never reached the app, so it holds no session of this spec's.
  }
  try {
    const list = await apiGet<{ providers: { id: number }[] }>(browser, "/api/providers");
    for (const provider of list.providers) await apiSend(browser, "DELETE", `/api/providers/${provider.id}`);
  } catch {
    // The page never reached the app, so it saved no provider of this spec's.
  }
});

test("the settings screens show the offline state and delete what this spec created", async ({ app, browser, screen }) => {
  // The learner reads in English, which is the language these strings are read
  // in. Nothing else is changed: the settings are the state a learner has.
  await browser.addInitScript(() => {
    try {
      const stored = JSON.parse(localStorage.getItem("lumingo.prefs.v1") ?? "{}") as Record<string, unknown>;
      localStorage.setItem("lumingo.prefs.v1", JSON.stringify({ ...stored, language: "en" }));
    } catch {
      // Storage can be blocked; the page works without it.
    }
  });
  await browser.setViewport(TALL_VIEWPORT);

  // 1. The screen is reached through the menu, which is what proves the nav
  //    entry end to end, and the three settings pages are linked from it.
  await app.open("/unit/");
  await shellSettled(browser);
  await tapAt(screen, screen.getByRole("link", { name: TEXT.navSettings }));
  await expect(browser).toHaveURL("/settings/", { timeout: 15_000 });
  await shellSettled(browser);
  await expect(screen.getByRole("heading", { level: 1, name: TEXT.title })).toBeVisible();
  const subnav = screen.getByRole("navigation", { name: TEXT.navLabel });
  await expect(subnav.getByRole("link", { name: TEXT.navGeneral })).toBeVisible();
  await expect(subnav.getByRole("link", { name: TEXT.navPrivacy })).toBeVisible();
  await expect(subnav.getByRole("link", { name: TEXT.navDiagnostics })).toBeVisible();

  // The provider-call counter as this walk found it. Nothing this spec does
  // before the diagnostics page is read may add a call, and that is what the
  // baseline makes provable instead of assumed.
  const baseline = await apiGet<DiagnosticsReport>(browser, "/api/diagnostics");

  // 2. General: a fresh data directory has no provider profile, and the panel
  //    offers the control that adds one rather than pretending one exists.
  await expect(screen.getByRole("heading", { name: TEXT.providersTitle })).toBeVisible();
  await expect(screen.getByText(TEXT.providerEmpty)).toBeVisible();
  const addProfile = screen.getByRole("button", { name: TEXT.providersAdd });
  await expect(addProfile).toBeVisible();

  // 3. Display and language: the language this spec pinned is the one checked.
  await expect(screen.getByRole("heading", { name: TEXT.display })).toBeVisible();
  await expect(screen.getByRole("group", { name: TEXT.language }).getByRole("radio", { name: TEXT.languageEn })).toBeChecked();

  // 4. The learner's own settings: a name field and the Indonesian-help choice.
  await expect(screen.getByLabel(TEXT.name)).toBeVisible();
  await expect(screen.getByRole("group", { name: TEXT.l1help })).toBeVisible();

  // 5. Timing: three choices, `auto` stored, and the panel says plainly that the
  //    choice changes nothing in this build. Choosing another one saves the
  //    whole settings set through `PUT /api/settings`, and the control shows
  //    what the program stored afterwards.
  const timing = screen.getByRole("group", { name: TEXT.timingTitle });
  await expect(timing).toBeVisible();
  for (const label of [TEXT.timingAuto, TEXT.timingWait, TEXT.timingNever]) {
    await expect(timing.getByRole("radio", { name: label })).toBeVisible();
  }
  await expect(timing.getByRole("radio", { name: TEXT.timingAuto })).toBeChecked();
  await expect(screen.getByText(TEXT.timingEffect)).toBeVisible();
  await tapAt(screen, timing.getByRole("radio", { name: TEXT.timingNever }));
  await expect(timing.getByRole("radio", { name: TEXT.timingNever })).toBeChecked({ timeout: 15_000 });
  expect((await apiGet<SettingsReport>(browser, "/api/settings")).adaptive_timing).toBe("never_wait");

  // 6. Recordings: off, next to the sentence that says raw audio is not kept
  //    without it, and the hint says nothing is recorded in this build. Switching
  //    it on and off again saves both ways, and the server is read back each
  //    time so the proof is the stored answer rather than the control's own
  //    state.
  const recordings = screen.getByRole("switch", { name: TEXT.recordingsTitle });
  await expect(recordings).toHaveAttribute("aria-checked", "false");
  await expect(screen.getByText(TEXT.privacyAudio)).toBeVisible();
  await expect(screen.getByText(TEXT.recordingsHint)).toBeVisible();
  await tapAt(screen, recordings);
  await expect(recordings).toHaveAttribute("aria-checked", "true", { timeout: 15_000 });
  expect((await apiGet<SettingsReport>(browser, "/api/settings")).keep_recordings).toBe(true);
  await expect(recordings).toBeEnabled({ timeout: 15_000 });
  await tapAt(screen, recordings);
  await expect(recordings).toHaveAttribute("aria-checked", "false", { timeout: 15_000 });
  expect((await apiGet<SettingsReport>(browser, "/api/settings")).keep_recordings).toBe(false);

  // 7. Audio devices still has no route in this build, and the panel says so
  //    instead of drawing a control that would do nothing.
  await expect(screen.getByRole("heading", { name: TEXT.audioTitle })).toBeVisible();
  await expect(screen.getByText(TEXT.unavailableTitle)).toBeVisible();
  await expect(screen.getByText(TEXT.speechMissing)).toBeVisible();

  // 8. Models: the panel lists the shipped manifest the harness serves. Every
  //    row is one entry of that file — its id, its version, its licence and
  //    licence link — and the server's own reason it cannot be downloaded. No
  //    row offers a download control, because `files` is empty for every
  //    candidate: a real download needs checksums the owner fills in.
  await expect(screen.getByRole("heading", { name: TEXT.modelsTitle })).toBeVisible();
  expect(await screen.getByText(TEXT.modelsEmpty).count()).toBe(0);
  const modelRows = browser.locator("ul.models > li.models__item");
  await expect(modelRows).toHaveCount(MANIFEST.length, { timeout: 15_000 });
  for (const [index, entry] of MANIFEST.entries()) {
    // The premise of the honest state is in the file itself, so it is asserted
    // rather than assumed: every candidate lists no files.
    expect(entry.files).toBe("[]");
    const row = modelRows.nth(index);
    await expect(row.getByRole("heading", { name: entry.id })).toBeVisible();
    await expect(row.getByText(TEXT.modelsVersion(entry.version))).toBeVisible();
    await expect(row.getByText(TEXT.modelsLicence(entry.license))).toBeVisible();
    if (entry.licenseUrl === "") {
      // Nothing has been read at the source, so the panel offers no link to it.
      expect(await row.getByRole("link", { name: TEXT.licenceOpen }).count()).toBe(0);
    } else {
      await expect(row.getByRole("link", { name: TEXT.licenceOpen })).toHaveAttribute("href", entry.licenseUrl);
    }
    await expect(row.getByText(TEXT.modelsWhy(cannotBeDownloaded(entry)))).toBeVisible();
    expect(await row.getByRole("button", { name: TEXT.modelsDownload }).count()).toBe(0);
  }
  expect(await screen.getByRole("button", { name: TEXT.modelsDownload }).count()).toBe(0);

  // 9. The pinned fake provider is typed in through the form. Its base URL is a
  //    closed loopback port, so the program accepts the profile and nothing ever
  //    connects. After the save the card shows the last four characters of the
  //    key and nothing else — and no part of the key itself is anywhere on the
  //    page, which is scanned as a whole rather than only the card.
  await tapAt(screen, addProfile);
  await expect(screen.getByLabel(TEXT.providerName)).toBeVisible({ timeout: 15_000 });
  await screen.getByLabel(TEXT.providerName).fill(PROVIDER.name);
  await expect(
    screen.getByRole("group", { name: TEXT.providerProtocol }).getByRole("radio", { name: TEXT.providerProtocolOpenai }),
  ).toBeChecked();
  await screen.getByLabel(TEXT.providerBaseUrl).fill(PROVIDER.baseUrl);
  await screen.getByLabel(TEXT.providerModel).fill(PROVIDER.model);
  await screen.getByLabel(TEXT.providerKey).fill(PROVIDER.key);
  const saveProfile = screen.getByRole("button", { name: TEXT.providerSave });
  await expect(saveProfile).toBeEnabled();
  await tapAt(screen, saveProfile);
  const card = screen.getByRole("article", { name: PROVIDER.name });
  await expect(card).toBeVisible({ timeout: 15_000 });
  await expect(card.getByText(TEXT.keySaved(PROVIDER.last4))).toBeVisible();
  await expect(card.getByText(PROVIDER.baseUrl)).toBeVisible();
  await expect(card.getByText(PROVIDER.model)).toBeVisible();
  await expect(card.getByText(TEXT.testedNever)).toBeVisible();
  expect(await screen.getByText(TEXT.providerEmpty).count()).toBe(0);
  const saved = (await apiGet<{ providers: ProviderReport[] }>(browser, "/api/providers")).providers;
  expect(saved).toHaveLength(1);
  expect(saved[0]?.name).toBe(PROVIDER.name);
  expect(saved[0]?.key_last4).toBe(PROVIDER.last4);
  const pageText = (await browser.evaluate(() => document.body.textContent ?? "")) as string;
  expect(pageText).not.toContain(PROVIDER.middle);
  expect(pageText).not.toContain(PROVIDER.key);

  // 10. Diagnostics, read while the profile is saved and no session exists yet.
  //     Every fact is the one the server reported: the program's own version,
  //     mode, uptime, address and schema; what this computer measured against
  //     the floor; the active provider the profile step saved; the curriculum's
  //     one unit file; and the five latency rows with no samples.
  await tapAt(screen, screen.getByRole("link", { name: TEXT.navDiagnostics }));
  await expect(browser).toHaveURL("/settings/diagnostics/", { timeout: 15_000 });
  await shellSettled(browser);
  await expect(screen.getByRole("heading", { level: 1, name: TEXT.diagTitle })).toBeVisible();
  const report = await apiGet<DiagnosticsReport>(browser, "/api/diagnostics");

  const programPanel = panel(browser, TEXT.diagProgram);
  await expect(programPanel).toHaveCount(1);
  for (const fact of [TEXT.diagVersion, report.server_version, TEXT.diagMode, TEXT.diagSchema, String(report.schema_version)]) {
    await expect(programPanel).toContainText(fact);
  }
  await expect(programPanel).toContainText(report.dev_mode ? "Development" : TEXT.modeNormal);
  await expect(programPanel).toContainText(TEXT.diagUptime);
  await expect(programPanel).toContainText(/Running for\s+\d+ (h \d+ min|min \d+ s|s)/);
  await expect(programPanel).toContainText(TEXT.diagAddress);
  if (report.server_address !== null) await expect(programPanel).toContainText(report.server_address);

  const hardwarePanel = panel(browser, TEXT.diagHardware);
  await expect(hardwarePanel).toHaveCount(1);
  await expect(hardwarePanel).toContainText(TEXT.hardwareRam);
  await expect(hardwarePanel).toContainText(TEXT.hardwareCores);
  await expect(hardwarePanel).toContainText(TEXT.hardwareMinimum);
  const minimumRam = TEXT.gigabytes(gigabytes(report.hardware.minimum_ram_bytes));
  const minimumCores = report.hardware.minimum_logical_cores;
  await expect(hardwarePanel).toContainText(
    report.hardware.meets_minimum === null
      ? TEXT.hardwareUnknown(minimumRam, minimumCores)
      : report.hardware.meets_minimum
        ? TEXT.hardwareMeets(minimumRam, minimumCores)
        : TEXT.hardwareBelow(minimumRam, minimumCores),
  );

  // The provider the profile step saved is still the active one, and no probe
  // has passed, which the page says instead of inventing capabilities.
  expect(report.provider?.name).toBe(PROVIDER.name);
  expect(report.provider?.model).toBe(PROVIDER.model);
  const providerPanel = panel(browser, TEXT.diagProvider);
  await expect(providerPanel).toHaveCount(1);
  await expect(providerPanel).toContainText(PROVIDER.name);
  await expect(providerPanel).toContainText(PROVIDER.model);
  await expect(providerPanel).toContainText(TEXT.diagProviderUntested);

  // The curriculum is the example set the harness indexes: one unit file.
  expect(report.curriculum.unit_count).toBe(1);
  const curriculumPanel = panel(browser, TEXT.diagCurriculum);
  await expect(curriculumPanel).toHaveCount(1);
  await expect(curriculumPanel).toContainText(TEXT.diagUnits);
  await expect(curriculumPanel).toContainText(String(report.curriculum.unit_count));

  // No latency sample exists yet, so every row says `n/a` and the empty
  // sentence is not shown: the table itself is the honest state.
  expect(report.latency.length).toBeGreaterThan(0);
  expect(report.latency.every((row) => row.stats.count === 0)).toBe(true);
  expect(await screen.getByText(TEXT.latencyEmpty).count()).toBe(0);
  const latencyRows = browser.locator("table.table tbody tr");
  await expect(latencyRows).toHaveCount(report.latency.length);
  for (let index = 0; index < report.latency.length; index += 1) {
    const row = latencyRows.nth(index);
    await expect(row).toContainText(report.latency[index]?.metric ?? "");
    const cells = row.getByRole("cell");
    await expect(cells.nth(0)).toHaveText("0");
    await expect(cells.nth(1)).toHaveText(TEXT.latencyNone);
    await expect(cells.nth(2)).toHaveText(TEXT.latencyNone);
  }

  // Nothing this walk did has called a provider: saving a profile is not a
  // request, and browsing the screens sends nothing. The counter is therefore
  // where the baseline left it, which on a run of this spec alone is zero — and
  // then the panel says so in its own words.
  expect(report.llm.calls).toBe(baseline.llm.calls);
  expect(report.llm.failures).toBe(baseline.llm.failures);
  const llmPanel = panel(browser, TEXT.diagLlm);
  await expect(llmPanel).toHaveCount(1);
  if (report.llm.calls === 0) {
    await expect(llmPanel).toContainText(TEXT.diagLlmEmpty);
  } else {
    await expect(llmPanel).toContainText(TEXT.diagLlmCalls);
    await expect(llmPanel).toContainText(String(report.llm.calls));
  }

  // 11. Privacy: the statements PRD 12.4 wants, in the learner's language.
  await tapAt(screen, screen.getByRole("link", { name: TEXT.navPrivacy }));
  await expect(browser).toHaveURL("/settings/privacy/", { timeout: 15_000 });
  await shellSettled(browser);
  await expect(screen.getByRole("heading", { level: 1, name: TEXT.dataTitle })).toBeVisible();
  for (const statement of [TEXT.privacyVoice, TEXT.privacyText, TEXT.privacyKey, TEXT.privacyAudio, TEXT.privacyLocal]) {
    await expect(screen.getByText(statement)).toBeVisible();
  }

  // 12. The data locations are the ones the program reported, not a copy of
  //     them written into this spec.
  await expect(screen.getByText(TEXT.whereData)).toBeVisible();
  await expect(screen.getByText(report.data_dir)).toBeVisible();
  await expect(screen.getByText(TEXT.whereUnits)).toBeVisible();
  await expect(screen.getByText(report.curriculum_dir)).toBeVisible();
  await expect(screen.getByText(TEXT.whereAddress)).toBeVisible();
  if (report.server_address !== null) await expect(screen.getByText(report.server_address).first()).toBeVisible();

  // 13. The payload inspector is real and honest about its limits. It is empty:
  //     offline nothing has been sent, and saving a profile is not a request.
  //     The populated case is proved in Rust by `s6-10-key`.
  const inspector = await apiGet<InspectorReport>(browser, "/api/inspector");
  expect(inspector.entries).toHaveLength(0);
  await expect(screen.getByRole("heading", { name: TEXT.inspectorTitle })).toBeVisible();
  await expect(screen.getByText(TEXT.inspectorLimits(inspector.capacity, inspector.max_body_bytes))).toBeVisible();
  await expect(screen.getByText(TEXT.inspectorDropped(inspector.dropped))).toBeVisible();
  await expect(screen.getByText(TEXT.inspectorEmpty)).toBeVisible();

  // 14. The export is a real download: the control is tapped inside
  //     `waitForDownload`, and the file the runner kept is read from disk and
  //     parsed. It carries the profile's name and address and no part of the
  //     key — not the key, not its middle, and not even its last four
  //     characters.
  const download = await browser.waitForDownload(() =>
    tapAt(screen, screen.getByRole("button", { name: TEXT.exportButton })),
  );
  expect(download.suggestedFilename).toMatch(/^lumingo-export-\d{4}-\d{2}-\d{2}\.json$/);
  await expect(screen.getByText(TEXT.exportDone(download.suggestedFilename))).toBeVisible({ timeout: 15_000 });
  const exported = readFileSync(downloadOnDisk(download), "utf8");
  expect(exported).not.toContain(PROVIDER.key);
  expect(exported).not.toContain(PROVIDER.middle);
  expect(exported).not.toContain("key_last4");
  expect(exported).toContain(PROVIDER.name);
  expect(exported).toContain(PROVIDER.baseUrl);
  const bundle = JSON.parse(exported) as {
    format_version: number;
    data: { providers: { name: string; base_url: string; model: string }[] };
  };
  expect(bundle.format_version).toBe(1);
  expect(bundle.data.providers).toHaveLength(1);
  expect(bundle.data.providers[0]?.name).toBe(PROVIDER.name);
  expect(bundle.data.providers[0]?.base_url).toBe(PROVIDER.baseUrl);
  expect(bundle.data.providers[0]?.model).toBe(PROVIDER.model);

  // 15. One session is created only in order to be deleted: a writing session
  //     with the learner's own prompt needs no provider, and layer one — the
  //     word count and the rule checker's findings — comes back at once. It is
  //     finished through the screen's own control, so the server's single
  //     session slot is free afterwards.
  await app.open("/unit/");
  await shellSettled(browser);
  await tapAt(screen, screen.getByRole("link", { name: TEXT.navWrite }));
  await expect(browser).toHaveURL("/write/", { timeout: 15_000 });
  await shellSettled(browser);
  await expect(screen.getByRole("heading", { level: 1, name: TEXT.writeTitle })).toBeVisible();
  await screen.getByLabel(TEXT.writePrompt).fill(PROMPT);
  await tapAt(screen, screen.getByRole("button", { name: TEXT.writeStart }));
  await expect(screen.getByLabel(TEXT.writeDraft)).toBeVisible({ timeout: 15_000 });
  await screen.getByLabel(TEXT.writeDraft).fill(DRAFT);
  await tapAt(screen, screen.getByRole("button", { name: TEXT.writeCheck }));
  await expect(screen.getByRole("heading", { name: TEXT.writeRules })).toBeVisible({ timeout: 30_000 });
  await tapAt(screen, screen.getByRole("button", { name: TEXT.writeFinish }));
  await expect(screen.getByRole("heading", { name: TEXT.writeEnded })).toBeVisible({ timeout: 30_000 });
  expect(await activeSessionId(browser)).toBeNull();

  // 16. The privacy page's deleter lists the finished session, and the delete
  //     needs the session's own number typed. The row is gone afterwards and the
  //     outcome banner names what was removed.
  await app.open("/settings/privacy/");
  await shellSettled(browser);
  await expect(screen.getByRole("heading", { level: 1, name: TEXT.dataTitle })).toBeVisible();
  const progress = await apiGet<ProgressReport>(browser, "/api/progress");
  const created = progress.recent_sessions.find((session) => session.kind === "writing");
  if (created === undefined) throw new Error("the finished writing session is not listed in the recent sessions");
  await expect(screen.getByText(new RegExp(`^Session ${created.id}: ${TEXT.sessionKindWriting}, started `))).toBeVisible({
    timeout: 15_000,
  });
  await tapAt(screen, screen.getByRole("button", { name: TEXT.sessionDelete(created.id) }));
  await expect(screen.getByText(TEXT.sessionConfirm(created.id))).toBeVisible();
  await screen.getByLabel(TEXT.confirmLabel).fill(String(created.id));
  const confirmSession = screen.getByRole("button", { name: TEXT.confirmYes });
  await expect(confirmSession).toBeEnabled();
  await tapAt(screen, confirmSession);
  // The delete is answered only after the program has compacted the database —
  // `delete_session` folds the write-ahead log and vacuums so the deleted text
  // does not linger in free pages. That is the same work the wipe below does, and
  // it is bounded by the disk rather than by the screen: on a loaded machine it
  // has been measured at 22s, so it gets the same 30s the wipe's outcome gets
  // instead of the 15s a screen-local step would need.
  await expect(screen.getByText(TEXT.sessionDone(created.id))).toBeVisible({ timeout: 30_000 });
  await expect(screen.getByText(TEXT.audioRemoved(0)).first()).toBeVisible();
  await expect(screen.getByRole("button", { name: TEXT.sessionDelete(created.id) })).toHaveCount(0);

  // 17. Deleting everything asks for a typed phrase: the first tap only opens
  //     that question, and the confirm control stays off until the phrase
  //     matches. The learner's record is unchanged while it waits.
  const before = await apiGet<ProgressReport>(browser, "/api/progress");
  await tapAt(screen, screen.getByRole("button", { name: TEXT.allButton }));
  await expect(screen.getByText(TEXT.allConfirm(TEXT.allPhrase))).toBeVisible();
  await expect(screen.getByRole("button", { name: TEXT.confirmYes })).toBeDisabled();
  expect(await apiGet<ProgressReport>(browser, "/api/progress")).toEqual(before);

  // 18. With the phrase typed, the wipe runs and reports what it did.
  await screen.getByLabel(TEXT.confirmLabel).fill(TEXT.allPhrase);
  const confirmAll = screen.getByRole("button", { name: TEXT.confirmYes });
  await expect(confirmAll).toBeEnabled();
  await tapAt(screen, confirmAll);
  await expect(screen.getByText(TEXT.allDone)).toBeVisible({ timeout: 30_000 });
  expect(await screen.getByText(TEXT.audioRemoved(0)).count()).toBeGreaterThanOrEqual(1);

  // 19. The progress page is back to the state a fresh data directory has.
  await tapAt(screen, screen.getByRole("link", { name: TEXT.navProgress }));
  await expect(browser).toHaveURL("/progress/", { timeout: 15_000 });
  await shellSettled(browser);
  await expect(screen.getByRole("heading", { level: 1, name: TEXT.progressTitle })).toBeVisible();
  await expect(screen.getByText(TEXT.progressEmpty)).toBeVisible({ timeout: 15_000 });

  // 20. The pinned profile is deleted through the screen, so the server is left
  //     with no provider at all — the state `tests/talk.e2e.ts` asserts.
  await tapAt(screen, screen.getByRole("link", { name: TEXT.navSettings }));
  await expect(browser).toHaveURL("/settings/", { timeout: 15_000 });
  await shellSettled(browser);
  const savedCard = screen.getByRole("article", { name: PROVIDER.name });
  await expect(savedCard).toBeVisible({ timeout: 15_000 });
  await tapAt(screen, savedCard.getByRole("button", { name: TEXT.providerDelete }));
  await tapAt(screen, screen.getByRole("button", { name: TEXT.providerDeleteYes }));
  await expect(screen.getByText(TEXT.providerEmpty)).toBeVisible({ timeout: 15_000 });
  await expect(screen.getByRole("article", { name: PROVIDER.name })).toHaveCount(0);
  expect((await apiGet<{ providers: ProviderReport[] }>(browser, "/api/providers")).providers).toHaveLength(0);
});
