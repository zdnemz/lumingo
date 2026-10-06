"use client";

import { AppShell } from "@/components/AppShell";
import { ProgressScreen } from "@/progress/ProgressScreen";
import { useT } from "@/state/PreferencesProvider";

export default function ProgressPage() {
  const t = useT();
  return (
    <AppShell>
      <div className="px-stack" style={{ "--gap": "var(--space-5)" } as React.CSSProperties}>
        <h1>{t("progress.title")}</h1>
        <ProgressScreen />
      </div>
    </AppShell>
  );
}
