"use client";

import { AppShell } from "@/components/AppShell";
import { useT } from "@/state/PreferencesProvider";
import { TalkScreen } from "@/talk/TalkScreen";

export default function TalkPage() {
  const t = useT();
  return (
    <AppShell>
      <div className="px-stack" style={{ "--gap": "var(--space-5)" } as React.CSSProperties}>
        <h1>{t("talk.title")}</h1>
        <TalkScreen />
      </div>
    </AppShell>
  );
}
