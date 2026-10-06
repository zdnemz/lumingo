import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import OnboardingPage from "@/app/onboarding/page";
import DiagnosticsPage from "@/app/settings/diagnostics/page";
import PrivacyPage from "@/app/settings/privacy/page";
import SettingsPage from "@/app/settings/page";
import type { ProviderInfo } from "@/generated/ProviderInfo";
import type { SaveProviderRequest } from "@/generated/SaveProviderRequest";
import type { Settings } from "@/generated/Settings";
import {
  CAPS,
  DIAGNOSTICS,
  ENV_PROVIDER,
  FILE_PROVIDER,
  PROBE_OK,
  SESSIONS,
  SNAPSHOT,
  progressWith,
  providerList,
  renderApp,
  resetBrowserState,
  setSystem,
} from "@/test/utils";

const router = vi.hoisted(() => ({ push: vi.fn(), replace: vi.fn() }));
vi.mock("next/navigation", () => ({
  useRouter: () => router,
  usePathname: () => "/",
}));

/**
 * These tests run the real screens over the real typed client, with only the
 * network replaced, and record every request the page makes. They prove that
 * the new code paths talk to the page's own origin and only to routes that
 * `apps/server` has.
 */

/** The routes of apps/server/src/routes/mod.rs that these screens may call. */
const ROUTES: readonly { method: string; pattern: RegExp }[] = [
  { method: "GET", pattern: /^\/api\/state$/ },
  { method: "GET", pattern: /^\/api\/units$/ },
  { method: "GET", pattern: /^\/api\/providers$/ },
  { method: "POST", pattern: /^\/api\/providers$/ },
  { method: "DELETE", pattern: /^\/api\/providers\/\d+$/ },
  { method: "POST", pattern: /^\/api\/providers\/\d+\/test$/ },
  { method: "POST", pattern: /^\/api\/providers\/\d+\/activate$/ },
  { method: "GET", pattern: /^\/api\/progress$/ },
  { method: "GET", pattern: /^\/api\/game$/ },
  { method: "GET", pattern: /^\/api\/attempts\/\d+\/evidence$/ },
  { method: "GET", pattern: /^\/api\/settings$/ },
  { method: "PUT", pattern: /^\/api\/settings$/ },
  { method: "DELETE", pattern: /^\/api\/sessions\/\d+$/ },
  { method: "DELETE", pattern: /^\/api\/data$/ },
  { method: "GET", pattern: /^\/api\/export$/ },
  { method: "GET", pattern: /^\/api\/diagnostics$/ },
];

interface Seen {
  method: string;
  url: string;
}

let requests: Seen[] = [];
let sockets: string[] = [];
let blobUrls: string[] = [];
let providers: ProviderInfo[] = [];
let settings: Settings;

function json(body: unknown, init: ResponseInit = {}): Response {
  return new Response(JSON.stringify(body), { status: 200, headers: { "Content-Type": "application/json" }, ...init });
}

function backend(url: string, init?: RequestInit): Response {
  const method = init?.method ?? "GET";
  requests.push({ method, url });
  const body = typeof init?.body === "string" ? (JSON.parse(init.body) as unknown) : undefined;
  if (method === "GET" && url === "/api/state") return json({ ...SNAPSHOT, provider: providers.find((p) => p.is_active) ?? null, unavailable: ["sessions", "speech", "models"] });
  if (method === "GET" && url === "/api/units") return json({ content_version: "v1", units: [], issues: [] });
  if (method === "GET" && url === "/api/providers") return json(providerList(providers));
  if (method === "POST" && url === "/api/providers") {
    const request = body as SaveProviderRequest;
    const saved: ProviderInfo = {
      ...FILE_PROVIDER,
      id: 40 + providers.length,
      name: request.name,
      base_url: request.base_url,
      model: request.model,
      key_last4: request.api_key?.slice(-4) ?? null,
      is_active: request.make_active === true,
    };
    providers = [...providers.map((p) => ({ ...p, is_active: saved.is_active ? false : p.is_active })), saved];
    return json(saved);
  }
  if (method === "DELETE" && /^\/api\/providers\/\d+$/.test(url)) {
    providers = providers.filter((p) => `/api/providers/${p.id}` !== url);
    return json(providerList(providers));
  }
  if (method === "POST" && /\/test$/.test(url)) {
    providers = providers.map((p) => (url.includes(`/${p.id}/`) ? { ...p, probed_at: "2026-10-06T10:00:00Z", capabilities: CAPS } : p));
    return json(PROBE_OK);
  }
  if (method === "POST" && /\/activate$/.test(url)) {
    providers = providers.map((p) => ({ ...p, is_active: url.includes(`/${p.id}/`) }));
    return json(providers.find((p) => p.is_active));
  }
  if (method === "GET" && url === "/api/progress") return json(progressWith(SESSIONS));
  if (method === "GET" && url === "/api/settings") return json(settings);
  if (method === "PUT" && url === "/api/settings") {
    settings = body as Settings;
    return json(settings);
  }
  if (method === "DELETE" && /^\/api\/sessions\/\d+$/.test(url)) return json({ audio_files_removed: 0, compacted: true });
  if (method === "DELETE" && url === "/api/data") return json({ audio_files_removed: 0, compacted: true });
  if (method === "GET" && url === "/api/export") {
    return json({ format_version: 1, exported_at: "2026-10-06T00:00:00Z", app_version: "9.9.9", data: {} }, { headers: { "Content-Type": "application/json", "Content-Disposition": 'attachment; filename="lumingo-export-2026-10-06.json"' } });
  }
  if (method === "GET" && url === "/api/diagnostics") return json(DIAGNOSTICS);
  return json({ error: "not_found", message: "there is no such API route" }, { status: 404 });
}

class FakeSocket {
  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onmessage: ((message: { data: string }) => void) | null = null;
  constructor(url: string) {
    sockets.push(url);
    queueMicrotask(() => {
      this.onopen?.();
      this.onmessage?.({ data: JSON.stringify({ type: "Snapshot", seq: 0, state: { ...SNAPSHOT, provider: providers.find((p) => p.is_active) ?? null, unavailable: ["sessions", "speech", "models"] } }) });
    });
  }
  close() {}
}

beforeEach(() => {
  resetBrowserState();
  setSystem({ language: "en-US" });
  requests = [];
  sockets = [];
  blobUrls = [];
  providers = [];
  settings = { display_name: "Sari", ui_language: "en", l1: "id", l1_help_mode: "auto", adaptive_timing: "auto", keep_recordings: false };
  router.push.mockClear();
  router.replace.mockClear();
  vi.stubGlobal("fetch", vi.fn((url: string, init?: RequestInit) => Promise.resolve(backend(url, init))));
  vi.stubGlobal("WebSocket", FakeSocket);
  vi.stubGlobal("XMLHttpRequest", class {
    constructor() {
      throw new Error("XMLHttpRequest must not be used");
    }
  });
  vi.stubGlobal("navigator", Object.assign(Object.create(window.navigator), { sendBeacon: () => { throw new Error("sendBeacon must not be used"); } }));
  URL.createObjectURL = () => {
    const url = `blob:${window.location.origin}/${blobUrls.length}`;
    blobUrls.push(url);
    return url;
  };
  URL.revokeObjectURL = () => undefined;
  vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => undefined);
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

/** A WebSocket address is on this origin when it is the same host and port as the page. */
function sameOrigin(address: string): boolean {
  const url = new URL(address, window.location.href);
  const origin = url.protocol === "ws:" || url.protocol === "wss:" ? url.origin.replace(/^ws/, "http") : url.origin;
  return origin === window.location.origin;
}

/** Everything a page could load or navigate to by itself. Plain links the learner follows are checked apart. */
function externalResources(): string[] {
  const loaded = document.querySelectorAll("[src], link[href], form[action], iframe, object, embed, script, img, video, audio, source");
  return [...loaded].flatMap((element) => {
    const address = element.getAttribute("src") ?? element.getAttribute("href") ?? element.getAttribute("action");
    return address !== null && !sameOrigin(address) ? [`${element.tagName} ${address}`] : [];
  });
}

function expectOnlyOwnOriginAndRealRoutes() {
  expect(requests.length).toBeGreaterThan(0);
  for (const request of requests) {
    expect(request.url.startsWith("/"), `${request.method} ${request.url} is not a path on this origin`).toBe(true);
    expect(request.url.startsWith("//"), request.url).toBe(false);
    expect(
      ROUTES.some((route) => route.method === request.method && route.pattern.test(request.url)),
      `${request.method} ${request.url} is not a route of the server`,
    ).toBe(true);
  }
  for (const socket of sockets) expect(sameOrigin(socket), socket).toBe(true);
  for (const blob of blobUrls) expect(sameOrigin(blob), blob).toBe(true);
  expect(externalResources()).toEqual([]);
}

describe("requests made by the setup, settings, privacy and diagnostics screens", () => {
  it("the setup wizard", async () => {
    const user = userEvent.setup();
    renderApp(<OnboardingPage />);
    await screen.findByRole("heading", { name: "Welcome to Lumingo" });
    await user.click(screen.getByRole("button", { name: "Next" }));
    await user.click(await screen.findByRole("radio", { name: "Bahasa Indonesia" }));
    await waitFor(() => expect(settings.ui_language).toBe("id"));
    await user.click(screen.getByRole("radio", { name: "English" }));
    await waitFor(() => expect(settings.ui_language).toBe("en"));
    await user.click(screen.getByRole("button", { name: "Next" }));
    await user.click(screen.getByRole("button", { name: "Next" }));
    await user.click(screen.getByRole("button", { name: "Fill in these values" }));
    await user.click(screen.getByRole("button", { name: "Next" }));
    await user.type(screen.getByLabelText("API key"), "test-key-123456");
    await user.click(screen.getByRole("button", { name: "Save profile" }));
    await screen.findByText("Profile gemini-free saved.");
    await user.click(screen.getByRole("button", { name: "Next" }));
    await user.click(screen.getByRole("button", { name: "Run the test" }));
    await screen.findByText("The connection works");
    await waitFor(() => expect(screen.getByRole("button", { name: "Next" })).toBeEnabled());
    await user.click(screen.getByRole("button", { name: "Next" }));
    await user.click(screen.getByRole("button", { name: "Next" }));
    await screen.findByRole("heading", { name: "You are ready" });
    await screen.findByText("No unit files found yet.");

    expectOnlyOwnOriginAndRealRoutes();
    expect(sockets).toHaveLength(1);
    // The key was sent once, to the program on this origin.
    expect(requests.filter((r) => r.method === "POST" && r.url === "/api/providers")).toHaveLength(1);
  });

  it("settings: providers, language, name", async () => {
    const user = userEvent.setup();
    providers = [{ ...ENV_PROVIDER, is_active: true }, { ...FILE_PROVIDER, id: 5, is_active: false }];
    renderApp(<SettingsPage />);
    const mine = await screen.findByRole("article", { name: "my-gemini" });
    await user.click(within(mine).getByRole("button", { name: "Use this one" }));
    await waitFor(() => expect(within(screen.getByRole("article", { name: "my-gemini" })).getByText("In use")).toBeInTheDocument());
    await user.click(within(screen.getByRole("article", { name: "my-gemini" })).getByRole("button", { name: "Test the connection" }));
    await screen.findByText("The connection works");
    await user.click(within(screen.getByRole("article", { name: "my-gemini" })).getByRole("button", { name: "Delete" }));
    await user.click(screen.getByRole("button", { name: "Yes, delete it" }));
    await waitFor(() => expect(screen.queryByRole("article", { name: "my-gemini" })).toBeNull());
    await user.click(await screen.findByRole("radio", { name: "Bahasa Indonesia" }));
    await waitFor(() => expect(settings.ui_language).toBe("id"));

    expectOnlyOwnOriginAndRealRoutes();
  });

  it("privacy: export, delete a session, delete everything", async () => {
    const user = userEvent.setup();
    renderApp(<PrivacyPage />);
    await user.click(await screen.findByRole("button", { name: "Download my data" }));
    await screen.findByText(/lumingo-export-2026-10-06\.json/);
    await user.click(await screen.findByRole("button", { name: "Delete session 7" }));
    await user.type(screen.getByLabelText("Confirmation"), "7");
    await user.click(screen.getAllByRole("button", { name: "Delete" }).find((b) => !(b as HTMLButtonElement).disabled) as HTMLElement);
    await screen.findByText("Session 7 was deleted.");
    await user.click(screen.getByRole("button", { name: "Delete everything" }));
    await user.type(screen.getByLabelText("Confirmation"), "delete everything");
    await user.click(screen.getAllByRole("button", { name: "Delete" }).find((b) => !(b as HTMLButtonElement).disabled) as HTMLElement);
    await screen.findByText("Everything was deleted.");

    expectOnlyOwnOriginAndRealRoutes();
    expect(blobUrls).toHaveLength(1);
    // Each delete went to the one route it belongs to, once.
    expect(requests.filter((r) => r.method === "DELETE").map((r) => r.url)).toEqual(["/api/sessions/7", "/api/data"]);
  });

  it("diagnostics", async () => {
    renderApp(<DiagnosticsPage />);
    await screen.findByText("abc123");
    expectOnlyOwnOriginAndRealRoutes();
  });

  it("the only link that leaves this origin is a plain link the learner follows, and it carries no referrer", async () => {
    const user = userEvent.setup();
    renderApp(<OnboardingPage />);
    await screen.findByRole("heading", { name: "Welcome to Lumingo" });
    await user.click(screen.getByRole("button", { name: "Next" }));
    await user.click(screen.getByRole("button", { name: "Next" }));
    await user.click(screen.getByRole("button", { name: "Next" }));
    const link = await screen.findByRole("link", { name: "Open Google AI Studio" });
    expect(sameOrigin(link.getAttribute("href") ?? "")).toBe(false);
    expect(link.getAttribute("rel")).toBe("noopener noreferrer");
    expect(link.getAttribute("target")).toBe("_blank");
    expect(link.tagName).toBe("A");
    // No request went to that address.
    expect(requests.every((r) => r.url.startsWith("/api/"))).toBe(true);
  });
});
