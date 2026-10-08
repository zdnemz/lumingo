"use client";

import { AppShell } from "@/components/AppShell";
import { GeneralSettings } from "@/settings/GeneralSettings";
import { SettingsNav } from "@/settings/SettingsNav";
import { useT } from "@/state/PreferencesProvider";

export default function SettingsPage() {
  const t = useT();
  return (
    <AppShell>
      <div className="px-stack" style={{ "--gap": "var(--space-5)" } as React.CSSProperties}>
        <h1>{t("settings.title")}</h1>
        <SettingsNav current="general" />
        <GeneralSettings />
      </div>
    </AppShell>
  );
}
