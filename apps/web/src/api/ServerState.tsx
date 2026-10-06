"use client";

import { createContext, useContext, type ReactNode } from "react";
import { useEventStream, type EventStreamState } from "./useEventStream";

const ServerStateContext = createContext<EventStreamState | null>(null);

/**
 * One connection to the event stream for the whole page. Screens read the
 * latest snapshot and the connection status from here instead of each opening
 * their own socket.
 */
export function ServerStateProvider({ children }: { children: ReactNode }) {
  const state = useEventStream();
  return <ServerStateContext.Provider value={state}>{children}</ServerStateContext.Provider>;
}

export function useServerState(): EventStreamState {
  const value = useContext(ServerStateContext);
  if (value === null) throw new Error("useServerState must be used inside ServerStateProvider");
  return value;
}
