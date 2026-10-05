// The only place the UI talks to the server. Everything else gets an
// `ApiClient`, so screens can be tested against a fake one.

import type { ServerEvent } from "@/generated/ServerEvent";
import type { StateSnapshot } from "@/generated/StateSnapshot";

export class ApiError extends Error {
  readonly status: number;
  readonly code: string;

  constructor(status: number, code: string) {
    super(`request refused: ${status} ${code}`);
    this.name = "ApiError";
    this.status = status;
    this.code = code;
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

export interface ApiClient {
  getState: (signal?: AbortSignal) => Promise<StateSnapshot>;
  openEvents: (handlers: EventStreamHandlers) => EventStreamHandle;
}

/** Everything the server may send on the event stream. */
const EVENT_TYPES: ReadonlySet<string> = new Set<ServerEvent["type"]>(["Snapshot", "Heartbeat"]);

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

  return {
    async getState(signal) {
      await ensureSession();
      const response = await fetch(`${base}/api/state`, { credentials: "include", signal });
      if (!response.ok) {
        const body: unknown = await response.json().catch(() => null);
        const code =
          typeof body === "object" && body !== null && "error" in body && typeof body.error === "string"
            ? body.error
            : "unknown";
        throw new ApiError(response.status, code);
      }
      return (await response.json()) as StateSnapshot;
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
