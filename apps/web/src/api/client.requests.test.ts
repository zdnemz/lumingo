import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError, createApiClient, isUnreachable } from "./client";

interface Seen {
  url: string;
  method: string;
  headers: Record<string, string>;
  body: string | undefined;
  credentials: RequestCredentials | undefined;
}

let seen: Seen[] = [];

function answer(body: unknown, init: ResponseInit = {}): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "Content-Type": "application/json" },
    ...init,
  });
}

function stubFetch(respond: (url: string) => Response | Promise<Response>) {
  vi.stubGlobal(
    "fetch",
    vi.fn((url: string, init?: RequestInit) => {
      seen.push({
        url,
        method: init?.method ?? "GET",
        headers: (init?.headers ?? {}) as Record<string, string>,
        body: typeof init?.body === "string" ? init.body : undefined,
        credentials: init?.credentials,
      });
      return Promise.resolve(respond(url));
    }),
  );
}

beforeEach(() => {
  seen = [];
});
afterEach(() => {
  vi.unstubAllGlobals();
});

describe("typed client requests", () => {
  it("sends every route to a path on its own origin, with JSON on each change", async () => {
    stubFetch(() => answer({}));
    const api = createApiClient({ base: "" });
    await api.getState();
    await api.listProviders();
    await api.saveProvider({ name: "mine", protocol: "openai_chat", base_url: "https://x.example/v1", model: "m" });
    await api.deleteProvider(3);
    await api.testProvider(3);
    await api.activateProvider(3);
    await api.getSettings();
    await api.updateSettings({
      display_name: "A",
      ui_language: "en",
      l1: "id",
      l1_help_mode: "auto",
      adaptive_timing: "auto",
      keep_recordings: false,
    });
    await api.getDiagnostics();
    await api.getProgress();
    await api.listUnits();
    await api.getGame();
    await api.getAttemptEvidence(12);
    await api.deleteSession(7);
    await api.deleteAllData();
    await api.exportData();

    expect(seen.map((r) => `${r.method} ${r.url}`)).toEqual([
      "GET /api/state",
      "GET /api/providers",
      "POST /api/providers",
      "DELETE /api/providers/3",
      "POST /api/providers/3/test",
      "POST /api/providers/3/activate",
      "GET /api/settings",
      "PUT /api/settings",
      "GET /api/diagnostics",
      "GET /api/progress",
      "GET /api/units",
      "GET /api/game",
      "GET /api/attempts/12/evidence",
      "DELETE /api/sessions/7",
      "DELETE /api/data",
      "GET /api/export",
    ]);
    for (const request of seen) {
      expect(request.url.startsWith("/api/"), request.url).toBe(true);
      expect(request.credentials).toBe("include");
      if (request.method !== "GET") expect(request.headers["Content-Type"]).toBe("application/json");
    }
  });

  it("reads the error code and the details sentence of a refused request", async () => {
    stubFetch(() => answer({ error: "read_only", message: "the environment profile is read-only" }, { status: 409 }));
    const api = createApiClient({ base: "" });
    const error = await api.deleteProvider(1).catch((caught: unknown) => caught);
    expect(error).toBeInstanceOf(ApiError);
    expect(error).toMatchObject({ status: 409, code: "read_only", detail: "the environment profile is read-only" });
    expect(isUnreachable(error)).toBe(false);
  });

  it("treats a failed fetch as an unreachable program", async () => {
    vi.stubGlobal("fetch", () => Promise.reject(new TypeError("network")));
    const api = createApiClient({ base: "" });
    const error = await api.getSettings().catch((caught: unknown) => caught);
    expect(isUnreachable(error)).toBe(true);
    expect(isUnreachable(new DOMException("stop", "AbortError"))).toBe(false);
  });

  it("names the export from the server's header, and falls back safely", async () => {
    stubFetch(() =>
      answer({ data: {} }, { headers: { "Content-Disposition": 'attachment; filename="lumingo-export-2026-10-06.json"' } }),
    );
    const api = createApiClient({ base: "" });
    expect((await api.exportData()).filename).toBe("lumingo-export-2026-10-06.json");

    stubFetch(() => answer({}, { headers: { "Content-Disposition": 'attachment; filename="../../x"' } }));
    expect((await api.exportData()).filename).toBe("lumingo-export.json");
  });
});
