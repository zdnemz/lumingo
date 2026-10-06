import { act, render } from "@testing-library/react";
import type { ReactElement, ReactNode } from "react";
import { expect, vi } from "vitest";
import { ServerStateProvider } from "@/api/ServerState";
import type { ApiClient, EventStreamHandlers } from "@/api/client";
import { Providers } from "@/components/Providers";
import type { DiagnosticsReport } from "@/generated/DiagnosticsReport";
import type { ProgressOverview } from "@/generated/ProgressOverview";
import type { SessionSummary } from "@/generated/SessionSummary";
import type { ProbeFailureKind } from "@/generated/ProbeFailureKind";
import type { ProbeReport } from "@/generated/ProbeReport";
import type { ProviderCapabilities } from "@/generated/ProviderCapabilities";
import type { ProviderInfo } from "@/generated/ProviderInfo";
import type { ProviderList } from "@/generated/ProviderList";
import type { ServerEvent } from "@/generated/ServerEvent";
import type { StateSnapshot } from "@/generated/StateSnapshot";

/** jsdom has no matchMedia. This installs one whose answers the test controls. */
export function setSystem(options: { reduced?: boolean; dark?: boolean; language?: string }): void {
  const { reduced = false, dark = true, language = "en-US" } = options;
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    value: (query: string) => ({
      matches: query.includes("reduced-motion") ? reduced : dark,
      media: query,
      addEventListener: () => undefined,
      removeEventListener: () => undefined,
    }),
  });
  Object.defineProperty(window.navigator, "language", { configurable: true, value: language });
}

export function resetBrowserState(): void {
  window.localStorage.clear();
  const root = document.documentElement;
  delete root.dataset.motion;
  delete root.dataset.theme;
  delete root.dataset.crt;
  root.lang = "";
}

export const SNAPSHOT: StateSnapshot = {
  server_version: "9.9.9",
  dev_mode: false,
  uptime_ms: 5000,
  settings: {
    display_name: "Learner",
    ui_language: "en",
    l1: "id",
    l1_help_mode: "auto",
    adaptive_timing: "auto",
    keep_recordings: false,
  },
  provider: null,
  hardware: {
    ram_total_bytes: 16_000_000_000,
    logical_cores: 8,
    minimum_ram_bytes: 8_000_000_000,
    minimum_logical_cores: 4,
    meets_minimum: true,
  },
  unavailable: [],
};

export const CAPS: ProviderCapabilities = {
  probe_version: 1,
  auth_ok: true,
  stream_ok: true,
  ttft_ms: 420,
  tokens_per_second: 55,
  structured_level: 1,
  contracts_ok: ["t1", "t2"],
  rate_limit_rpm: 15,
  rate_limit_rpd: null,
};

export const FILE_PROVIDER: ProviderInfo = {
  id: 2,
  name: "my-gemini",
  protocol: "openai_chat",
  base_url: "https://api.example.test/v1",
  model: "tutor-model",
  has_key: true,
  key_last4: "4321",
  source: "file",
  is_active: true,
  capabilities: null,
  probed_at: null,
  qualified_at: null,
};

export const ENV_PROVIDER: ProviderInfo = {
  ...FILE_PROVIDER,
  id: 1,
  name: "env",
  source: "env",
  key_last4: "9876",
  is_active: false,
};

export function providerList(providers: ProviderInfo[], problems: string[] = []): ProviderList {
  return { providers, active_id: providers.find((p) => p.is_active)?.id ?? null, problems };
}

export const PROBE_OK: ProbeReport = { provider_id: 2, ok: true, capabilities: CAPS, failure: null, duration_ms: 2300 };

export function probeFailure(kind: ProbeFailureKind): ProbeReport {
  return { provider_id: 2, ok: false, capabilities: null, failure: { kind, message: `test failure: ${kind}` }, duration_ms: 900 };
}

export const DIAGNOSTICS: DiagnosticsReport = {
  server_version: "9.9.9",
  dev_mode: false,
  uptime_ms: 3_725_000,
  server_address: "127.0.0.1:8765",
  data_dir: "C:/Users/Sari/AppData/Roaming/Lumingo",
  log_folder: null,
  curriculum_dir: "C:/Lumingo/curriculum/units",
  schema_version: 1,
  hardware: SNAPSHOT.hardware,
  provider: { ...FILE_PROVIDER, capabilities: CAPS, probed_at: "2026-10-06T10:00:00Z" },
  curriculum: { unit_count: 12, issue_count: 1, content_version: "abc123" },
  latency: [
    { metric: "stt_ms", stats: { count: 4, p50_ms: 410, p95_ms: 690 } },
    { metric: "e2e_ms", stats: { count: 0, p50_ms: null, p95_ms: null } },
  ],
  llm: {
    calls: 6,
    failures: 1,
    ttft: { count: 5, p50_ms: 500, p95_ms: 900 },
    total: { count: 5, p50_ms: 2100, p95_ms: 3000 },
  },
};

export const SESSIONS: SessionSummary[] = [
  { id: 7, kind: "conversation", unit_id: "a1-u01", status: "completed", started_at: "2026-10-05T09:00:00Z", ended_at: "2026-10-05T09:20:00Z" },
  { id: 8, kind: "writing", unit_id: null, status: "aborted", started_at: "2026-10-06T08:00:00Z", ended_at: null },
];

export function progressWith(sessions: SessionSummary[]): ProgressOverview {
  return {
    units: [],
    objectives: [],
    errors: [],
    reviews_due: [],
    reviews_due_truncated: false,
    estimates: [],
    recent_sessions: sessions,
  };
}

export interface FakeApi extends ApiClient {
  emit: (event: ServerEvent) => void;
  open: () => void;
  drop: () => void;
  opened: () => number;
}

/** A route a test did not stub fails loudly, so a screen cannot quietly depend on it. */
function notStubbed(route: string): () => Promise<never> {
  return () => Promise.reject(new Error(`route not stubbed in this test: ${route}`));
}

/**
 * A typed client whose stream the test drives by hand. Routes other than
 * `getState` fail unless the test stubs them through `overrides`.
 */
export function fakeApi(
  getState: () => Promise<StateSnapshot> = () => Promise.resolve(SNAPSHOT),
  overrides: Partial<ApiClient> = {},
): FakeApi {
  let handlers: EventStreamHandlers | null = null;
  let opened = 0;
  const routes: Omit<ApiClient, "getState" | "openEvents"> = {
    listProviders: notStubbed("listProviders"),
    saveProvider: notStubbed("saveProvider"),
    deleteProvider: notStubbed("deleteProvider"),
    testProvider: notStubbed("testProvider"),
    activateProvider: notStubbed("activateProvider"),
    getSettings: notStubbed("getSettings"),
    updateSettings: notStubbed("updateSettings"),
    getDiagnostics: notStubbed("getDiagnostics"),
    getProgress: notStubbed("getProgress"),
    listUnits: notStubbed("listUnits"),
    deleteSession: notStubbed("deleteSession"),
    deleteAllData: notStubbed("deleteAllData"),
    exportData: notStubbed("exportData"),
  };
  return {
    ...routes,
    ...overrides,
    getState: vi.fn(getState),
    openEvents: vi.fn((next: EventStreamHandlers) => {
      handlers = next;
      opened += 1;
      return { close: () => undefined };
    }),
    emit: (event) => handlers?.onEvent(event),
    open: () => handlers?.onOpen(),
    drop: () => handlers?.onClose(),
    opened: () => opened,
  };
}

/** Renders inside the app providers. The wrapper stays in place on `rerender`. */
export function renderApp(ui: ReactElement, client?: ApiClient) {
  return render(ui, {
    wrapper: ({ children }: { children: ReactNode }) => <Providers client={client}>{children}</Providers>,
  });
}

/** Renders a screen that reads the shared event stream, as the app shell provides it. */
export function renderScreen(ui: ReactElement, client: ApiClient) {
  return renderApp(<ServerStateProvider>{ui}</ServerStateProvider>, client);
}

/** Opens the fake stream and delivers the first snapshot, as the real server does on connect. */
export async function connect(api: FakeApi, state: StateSnapshot = SNAPSHOT): Promise<void> {
  await vi.waitFor(() => expect(api.opened()).toBeGreaterThan(0));
  act(() => {
    api.open();
    api.emit({ type: "Snapshot", seq: 0, state });
  });
}

/**
 * Every piece of text a learner can read in `root`: the text of elements that
 * hold only text, plus the accessible labels and placeholders. Used to prove a
 * language switch changes a whole screen.
 */
export function visibleStrings(root: HTMLElement): string[] {
  const found = new Set<string>();
  const add = (text: string | null | undefined) => {
    const value = text?.replace(/\s+/g, " ").trim();
    if (value) found.add(value);
  };
  for (const element of root.querySelectorAll("*")) {
    if (element.children.length === 0) add(element.textContent);
    add(element.getAttribute("aria-label"));
    add(element.getAttribute("placeholder"));
    add(element.getAttribute("title"));
  }
  return [...found];
}
