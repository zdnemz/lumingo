"use client";

import type { ReactNode } from "react";
import { ApiProvider } from "@/api/ApiProvider";
import type { ApiClient } from "@/api/client";
import { PreferencesProvider, usePreferences, useT } from "@/state/PreferencesProvider";
import { Mascot } from "@/sprites/Mascot";

/** Shown for the instant between the first paint and reading the stored preferences. */
function Boot({ children }: { children: ReactNode }) {
  const { ready } = usePreferences();
  const t = useT();
  if (ready) return <>{children}</>;
  return (
    <div className="boot px-sky" role="status">
      <Mascot mood="think" scale={6} />
      <p>{t("boot.loading")}</p>
    </div>
  );
}

export function Providers({ children, client }: { children: ReactNode; client?: ApiClient }) {
  return (
    <PreferencesProvider>
      <ApiProvider client={client}>
        <Boot>{children}</Boot>
      </ApiProvider>
    </PreferencesProvider>
  );
}
