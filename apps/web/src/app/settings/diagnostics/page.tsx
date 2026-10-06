"use client";

import { AppShell } from "@/components/AppShell";
import { DiagnosticsScreen } from "@/settings/DiagnosticsScreen";
import { SettingsNav } from "@/settings/SettingsNav";
import { useT } from "@/state/PreferencesProvider";

export default function DiagnosticsPage() {
  const t = useT();
  return (
    <AppShell>
      <div className="px-stack" style={{ "--gap": "var(--space-5)" } as React.CSSProperties}>
        <h1>{t("diag.title")}</h1>
        <SettingsNav current="diagnostics" />
        <DiagnosticsScreen />
      </div>
    </AppShell>
  );
}
