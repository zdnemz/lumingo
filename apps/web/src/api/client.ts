// The only place the UI talks to the server. Everything else gets an
// `ApiClient`, so screens can be tested against a fake one.

import type { AttemptEvidence } from "@/generated/AttemptEvidence";
import type { DeleteDataResult } from "@/generated/DeleteDataResult";
import type { DeleteSessionResult } from "@/generated/DeleteSessionResult";
import type { DiagnosticsReport } from "@/generated/DiagnosticsReport";
import type { GameState } from "@/generated/GameState";
import type { ProbeReport } from "@/generated/ProbeReport";
import type { ProgressOverview } from "@/generated/ProgressOverview";
import type { ProviderInfo } from "@/generated/ProviderInfo";
import type { ProviderList } from "@/generated/ProviderList";
import type { SaveProviderRequest } from "@/generated/SaveProviderRequest";
import type { ServerEvent } from "@/generated/ServerEvent";
import type { Settings } from "@/generated/Settings";
import type { StateSnapshot } from "@/generated/StateSnapshot";
import type { UnitList } from "@/generated/UnitList";

export class ApiError extends Error {
  readonly status: number;
  readonly code: string;
  /** The server's English sentence for a details line. Never the main message. */
  readonly detail: string;

  constructor(status: number, code: string, detail = "") {
    super(`request refused: ${status} ${code}`);
    this.name = "ApiError";
    this.status = status;
    this.code = code;
    this.detail = detail;
  }

  /** The server refuses requests that do not come from its own page. */
  get refused(): boolean {
    return this.status === 401 || this.status === 403;
  }
}

export interface EventStreamHandlers {
  onOpen: () => void;
  onEvent: (event: ServerEvent) => void;
  onClose: () => void;
}

export interface EventStreamHandle {
  close: () => void;
}

/** The export as text, with the file name the server gave the download. */
export interface ExportFile {
  filename: string;
  text: string;
}

/**
 * Every route of the server that the screens use. Only routes that exist in
 * `apps/server` are listed here.
 */
export interface ApiClient {
  getState: (signal?: AbortSignal) => Promise<StateSnapshot>;
  openEvents: (handlers: EventStreamHandlers) => EventStreamHandle;
  /** `GET /api/providers` */
  listProviders: (signal?: AbortSignal) => Promise<ProviderList>;
  /** `POST /api/providers` */
  saveProvider: (request: SaveProviderRequest) => Promise<ProviderInfo>;
  /** `DELETE /api/providers/{id}` */
  deleteProvider: (id: number) => Promise<ProviderList>;
  /** `POST /api/providers/{id}/test` */
  testProvider: (id: number) => Promise<ProbeReport>;
  /** `POST /api/providers/{id}/activate` */
  activateProvider: (id: number) => Promise<ProviderInfo>;
  /** `GET /api/settings` */
  getSettings: (signal?: AbortSignal) => Promise<Settings>;
  /** `PUT /api/settings`, which replaces every setting. */
  updateSettings: (settings: Settings) => Promise<Settings>;
  /** `GET /api/diagnostics` */
  getDiagnostics: (signal?: AbortSignal) => Promise<DiagnosticsReport>;
  /** `GET /api/progress` */
  getProgress: (signal?: AbortSignal) => Promise<ProgressOverview>;
  /** `GET /api/units` */
  listUnits: (signal?: AbortSignal) => Promise<UnitList>;
  /** `GET /api/game`. Cosmetic state only: it holds no attempt, evidence or estimate. */
  getGame: (signal?: AbortSignal) => Promise<GameState>;
  /** `GET /api/attempts/{id}/evidence` */
  getAttemptEvidence: (attemptId: number, signal?: AbortSignal) => Promise<AttemptEvidence>;
  /** `DELETE /api/sessions/{id}` */
  deleteSession: (id: number) => Promise<DeleteSessionResult>;
  /** `DELETE /api/data` */
  deleteAllData: () => Promise<DeleteDataResult>;
  /** `GET /api/export` */
  exportData: () => Promise<ExportFile>;
}

/** True when a request failed before any answer came back: the program is not running or not reachable. */
export function isUnreachable(error: unknown): boolean {
  if (error instanceof ApiError) return false;
  return !(error instanceof DOMException && error.name === "AbortError");
}

/** Everything the server may send on the event stream. */
const EVENT_TYPES: ReadonlySet<string> = new Set<ServerEvent["type"]>(["Snapshot", "Heartbeat", "ProviderStatus"]);

/** Accepts only messages that look like a known event, so a stray frame cannot crash a screen. */
export function parseServerEvent(text: string): ServerEvent | null {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return null;
  }
  if (typeof value !== "object" || value === null) return null;
  const record = value as Record<string, unknown>;
  if (typeof record.type !== "string" || !EVENT_TYPES.has(record.type)) return null;
  if (typeof record.seq !== "number") return null;
  return value as ServerEvent;
}

export interface ClientOptions {
  /**
   * Origin of the server when the page itself is not served by it, which only
   * happens in development under `next dev`. Empty means same origin.
   */
  base?: string;
}

/** Development only: the address of the running server, set in apps/web/.env.local. */
export function developmentBase(): string {
  return process.env.NEXT_PUBLIC_TUTOR_API ?? "";
}

/** Reads the `{"error": code, "message": text}` body of a refused request. */
async function refusal(response: Response): Promise<ApiError> {
  const body: unknown = await response.json().catch(() => null);
  const record = typeof body === "object" && body !== null ? (body as Record<string, unknown>) : {};
  const code = typeof record.error === "string" ? record.error : "unknown";
  const detail = typeof record.message === "string" ? record.message : "";
  return new ApiError(response.status, code, detail);
}

/** The file name of a download, when the server named one that is safe to use. */
function filenameOf(response: Response): string | null {
  const header = response.headers.get("content-disposition") ?? "";
  const match = /filename="([A-Za-z0-9._-]+)"/.exec(header);
  return match?.[1] ?? null;
}

interface RequestOptions {
  body?: unknown;
  signal?: AbortSignal;
}

export function createApiClient(options: ClientOptions = {}): ApiClient {
  const base = (options.base ?? developmentBase()).replace(/\/$/, "");
  let session: Promise<void> | null = null;

  /** `next dev` serves its own pages, so it asks the server for the session cookie once. */
  const ensureSession = (): Promise<void> => {
    if (base === "") return Promise.resolve();
    session ??= fetch(`${base}/dev/session`, { credentials: "include" }).then((response) => {
      if (!response.ok) throw new ApiError(response.status, "dev_session");
    });
    return session;
  };

  const eventsUrl = (): string => {
    const origin = base === "" ? window.location.origin : base;
    return `${origin.replace(/^http/, "ws")}/ws`;
  };

  /** One request. The server wants JSON on every change, so a request without a body still says so. */
  async function send(method: string, path: string, request: RequestOptions = {}): Promise<Response> {
    await ensureSession();
    const response = await fetch(`${base}${path}`, {
      method,
      credentials: "include",
      signal: request.signal,
      headers: method === "GET" ? undefined : { "Content-Type": "application/json" },
      body: request.body === undefined ? undefined : JSON.stringify(request.body),
    });
    if (!response.ok) throw await refusal(response);
    return response;
  }

  async function json<T>(method: string, path: string, request?: RequestOptions): Promise<T> {
    const response = await send(method, path, request);
    return (await response.json()) as T;
  }

  return {
    getState: (signal) => json<StateSnapshot>("GET", "/api/state", { signal }),
    listProviders: (signal) => json<ProviderList>("GET", "/api/providers", { signal }),
    saveProvider: (request) => json<ProviderInfo>("POST", "/api/providers", { body: request }),
    deleteProvider: (id) => json<ProviderList>("DELETE", `/api/providers/${id}`),
    testProvider: (id) => json<ProbeReport>("POST", `/api/providers/${id}/test`),
    activateProvider: (id) => json<ProviderInfo>("POST", `/api/providers/${id}/activate`),
    getSettings: (signal) => json<Settings>("GET", "/api/settings", { signal }),
    updateSettings: (settings) => json<Settings>("PUT", "/api/settings", { body: settings }),
    getDiagnostics: (signal) => json<DiagnosticsReport>("GET", "/api/diagnostics", { signal }),
    getProgress: (signal) => json<ProgressOverview>("GET", "/api/progress", { signal }),
    listUnits: (signal) => json<UnitList>("GET", "/api/units", { signal }),
    getGame: (signal) => json<GameState>("GET", "/api/game", { signal }),
    getAttemptEvidence: (attemptId, signal) =>
      json<AttemptEvidence>("GET", `/api/attempts/${encodeURIComponent(String(attemptId))}/evidence`, { signal }),
    deleteSession: (id) => json<DeleteSessionResult>("DELETE", `/api/sessions/${id}`),
    deleteAllData: () => json<DeleteDataResult>("DELETE", "/api/data"),
    async exportData() {
      const response = await send("GET", "/api/export");
      return { filename: filenameOf(response) ?? "lumingo-export.json", text: await response.text() };
    },

    openEvents(handlers) {
      let socket: WebSocket | null = null;
      let closed = false;
      void ensureSession()
        .catch(() => undefined)
        .then(() => {
          if (closed) return;
          socket = new WebSocket(eventsUrl());
          socket.onopen = handlers.onOpen;
          socket.onclose = handlers.onClose;
          socket.onmessage = (message: MessageEvent<unknown>) => {
            if (typeof message.data !== "string") return;
            const event = parseServerEvent(message.data);
            if (event !== null) handlers.onEvent(event);
          };
        });
      return {
        close() {
          closed = true;
          socket?.close();
        },
      };
    },
  };
}
