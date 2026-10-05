"use client";

import { AppShell } from "@/components/AppShell";
import { LinkStatus } from "@/components/LinkStatus";
import { Mascot } from "@/sprites/Mascot";
import { useT } from "@/state/PreferencesProvider";

export default function HomePage() {
  const t = useT();
  return (
    <AppShell>
      <div className="px-stack" style={{ "--gap": "var(--space-6)" } as React.CSSProperties}>
        <section className="hero" aria-labelledby="hero-title">
          <Mascot mood="happy" scale={8} />
          <div className="px-stack">
            <h1 id="hero-title">{t("app.name")}</h1>
            <p>{t("app.tagline")}</p>
          </div>
        </section>
        <LinkStatus />
      </div>
    </AppShell>
  );
}
