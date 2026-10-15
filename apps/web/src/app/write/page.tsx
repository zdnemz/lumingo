"use client";

import { AppShell } from "@/components/AppShell";
import { useT } from "@/state/PreferencesProvider";
import { WriteScreen } from "@/write/WriteScreen";

/**
 * The writing workshop. It needs no query string, so there is no Suspense
 * boundary, and it mounts no first-run gate: writing works with no provider
 * set up, which is the state a learner may well be in.
 */
export default function WritePage() {
  const t = useT();
  return (
    <AppShell>
      <div className="px-stack" style={{ "--gap": "var(--space-5)" } as React.CSSProperties}>
        <h1>{t("write.title")}</h1>
        <WriteScreen />
      </div>
    </AppShell>
  );
}
