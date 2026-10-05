"use client";

import { createContext, useContext, useMemo, type ReactNode } from "react";
import { createApiClient, type ApiClient } from "./client";

const ApiContext = createContext<ApiClient | null>(null);

/** Tests pass a fake client; the app creates the real one once. */
export function ApiProvider({ client, children }: { client?: ApiClient; children: ReactNode }) {
  const value = useMemo(() => client ?? createApiClient(), [client]);
  return <ApiContext.Provider value={value}>{children}</ApiContext.Provider>;
}

export function useApi(): ApiClient {
  const value = useContext(ApiContext);
  if (value === null) throw new Error("useApi must be used inside ApiProvider");
  return value;
}
