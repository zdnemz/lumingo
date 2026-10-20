"use client";

import { AboutScreen } from "@/about/AboutScreen";
import { AppShell } from "@/components/AppShell";
import { SettingsNav } from "@/settings/SettingsNav";
import { useT } from "@/state/PreferencesProvider";

export default function AboutPage() {
  const t = useT();
  return (
    <AppShell>
      <div className="px-stack" style={{ "--gap": "var(--space-5)" } as React.CSSProperties}>
        <h1>{t("about.title")}</h1>
        <SettingsNav current="about" />
        <AboutScreen />
      </div>
    </AppShell>
  );
}
