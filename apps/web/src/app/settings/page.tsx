"use client";

import { AppShell } from "@/components/AppShell";
import { DisplaySettings } from "@/components/DisplaySettings";
import { useT } from "@/state/PreferencesProvider";

export default function SettingsPage() {
  const t = useT();
  return (
    <AppShell>
      <div className="px-stack">
        <h1>{t("settings.title")}</h1>
        <DisplaySettings />
      </div>
    </AppShell>
  );
}
