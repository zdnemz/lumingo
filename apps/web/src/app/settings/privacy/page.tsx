"use client";

import { AppShell } from "@/components/AppShell";
import { PrivacyScreen } from "@/settings/PrivacyScreen";
import { SettingsNav } from "@/settings/SettingsNav";
import { useT } from "@/state/PreferencesProvider";

export default function PrivacyPage() {
  const t = useT();
  return (
    <AppShell>
      <div className="px-stack" style={{ "--gap": "var(--space-5)" } as React.CSSProperties}>
        <h1>{t("data.title")}</h1>
        <SettingsNav current="privacy" />
        <PrivacyScreen />
      </div>
    </AppShell>
  );
}
