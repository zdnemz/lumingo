"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import type { ServerEvent } from "@/generated/ServerEvent";
import type { StateSnapshot } from "@/generated/StateSnapshot";
import { useApi } from "./ApiProvider";
import { ApiError } from "./client";

export type StreamStatus = "connecting" | "live" | "lost" | "refused";

export interface EventStreamState {
  status: StreamStatus;
  /** The latest full state. A reconnect replaces it; nothing is replayed. */
  snapshot: StateSnapshot | null;
  /** Newest first, capped. */
  events: ServerEvent[];
  /** Tries to connect again now, for the "try again" button of the lost-connection banner. */
  retry: () => void;
}

type StreamData = Omit<EventStreamState, "retry">;

const KEEP_EVENTS = 8;
const RETRY_MS = [1000, 2000, 4000, 8000] as const;

/**
 * Connects to the server's event stream and keeps it connected. The first
 * message of every connection is a snapshot, which replaces what was shown.
 */
export function useEventStream(): EventStreamState {
  const api = useApi();
  const [state, setState] = useState<StreamData>({
    status: "connecting",
    snapshot: null,
    events: [],
  });
  const attempt = useRef(0);
  // Bumping this runs the connect effect again from a clean start.
  const [round, setRound] = useState(0);
  const retry = useCallback(() => {
    attempt.current = 0;
    setState((previous) => ({ ...previous, status: "connecting" }));
    setRound((value) => value + 1);
  }, []);

  useEffect(() => {
    let stopped = false;
    let retryTimer: ReturnType<typeof setTimeout> | undefined;
    let handle: { close: () => void } | undefined;
    const abort = new AbortController();

    const connect = () => {
      handle = api.openEvents({
        onOpen: () => {
          attempt.current = 0;
          setState((previous) => ({ ...previous, status: "live" }));
        },
        onEvent: (event) => {
          setState((previous) => ({
            status: "live",
            snapshot: applyEvent(previous.snapshot, event),
            events: [event, ...(event.type === "Snapshot" ? [] : previous.events)].slice(0, KEEP_EVENTS),
          }));
        },
        onClose: () => {
          if (stopped) return;
          setState((previous) => ({ ...previous, status: "lost" }));
          const delay = RETRY_MS[Math.min(attempt.current, RETRY_MS.length - 1)] ?? 8000;
          attempt.current += 1;
          retryTimer = setTimeout(connect, delay);
        },
      });
    };

    // A browser cannot read why a WebSocket handshake failed, so a plain request
    // first tells "the server refused this page" apart from "the server is gone".
    api
      .getState(abort.signal)
      .then(() => {
        if (!stopped) connect();
      })
      .catch((error: unknown) => {
        if (stopped) return;
        if (error instanceof ApiError && error.refused) {
          setState((previous) => ({ ...previous, status: "refused" }));
          return;
        }
        setState((previous) => ({ ...previous, status: "lost" }));
        retryTimer = setTimeout(connect, RETRY_MS[0]);
      });

    return () => {
      stopped = true;
      abort.abort();
      if (retryTimer !== undefined) clearTimeout(retryTimer);
      handle?.close();
    };
  }, [api, round]);

  return { ...state, retry };
}

/** A snapshot replaces everything. A provider change updates only the provider of what is shown. */
function applyEvent(snapshot: StateSnapshot | null, event: ServerEvent): StateSnapshot | null {
  if (event.type === "Snapshot") return event.state;
  if (event.type === "ProviderStatus" && snapshot !== null) return { ...snapshot, provider: event.provider };
  return snapshot;
}
