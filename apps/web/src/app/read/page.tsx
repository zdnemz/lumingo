"use client";

import { AppShell } from "@/components/AppShell";
import { ReadScreen } from "@/read/ReadScreen";
import { useT } from "@/state/PreferencesProvider";

export default function ReadPage() {
  const t = useT();
  return (
    <AppShell>
      <div className="px-stack" style={{ "--gap": "var(--space-5)" } as React.CSSProperties}>
        <h1>{t("read.title")}</h1>
        <ReadScreen />
      </div>
    </AppShell>
  );
}
