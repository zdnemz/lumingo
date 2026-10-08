"use client";

import { useCallback, useEffect, useState } from "react";

export type ResourceState<T> =
  | { status: "loading" }
  | { status: "error"; error: unknown }
  | { status: "ready"; data: T };

export interface Resource<T> {
  state: ResourceState<T>;
  /** Loads again, showing the loading state until the answer comes. */
  reload: () => void;
  /** Loads again in the background: what is shown stays until the new answer replaces it. */
  refresh: () => void;
  /** Replaces the loaded data, for a screen that already holds the newer answer. */
  replace: (data: T) => void;
}

/**
 * Loads one thing from the server when the screen opens. `load` must be
 * stable (wrap it in useCallback), because a new function loads again.
 */
export function useResource<T>(load: (signal: AbortSignal) => Promise<T>): Resource<T> {
  const [state, setState] = useState<ResourceState<T>>({ status: "loading" });
  const [round, setRound] = useState(0);

  useEffect(() => {
    const abort = new AbortController();
    load(abort.signal).then(
      (data) => {
        if (!abort.signal.aborted) setState({ status: "ready", data });
      },
      (error: unknown) => {
        if (!abort.signal.aborted) setState({ status: "error", error });
      },
    );
    return () => abort.abort();
  }, [load, round]);

  const reload = useCallback(() => {
    setState({ status: "loading" });
    setRound((value) => value + 1);
  }, []);
  const refresh = useCallback(() => setRound((value) => value + 1), []);
  const replace = useCallback((data: T) => setState({ status: "ready", data }), []);

  return { state, reload, refresh, replace };
}

export interface Action {
  busy: boolean;
  /** The error of the last run, or `null`. */
  error: unknown;
  /** Runs `task`. Resolves to its result, or `undefined` when it failed (the error is in `error`). */
  run: <T>(task: () => Promise<T>) => Promise<T | undefined>;
  clear: () => void;
}

/** A button-press that calls the server: tracks "busy" so it cannot run twice, and keeps the error. */
export function useAction(): Action {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);

  const run = useCallback(async <T,>(task: () => Promise<T>): Promise<T | undefined> => {
    setBusy(true);
    setError(null);
    try {
      return await task();
    } catch (caught) {
      setError(caught);
      return undefined;
    } finally {
      setBusy(false);
    }
  }, []);
  const clear = useCallback(() => setError(null), []);

  return { busy, error, run, clear };
}
