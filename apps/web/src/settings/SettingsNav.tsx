"use client";

import Link from "next/link";
import type { MessageKey } from "@/i18n";
import { useT } from "@/state/PreferencesProvider";

export type SettingsPage = "general" | "privacy" | "diagnostics";

const PAGES: readonly { id: SettingsPage; href: string; label: MessageKey }[] = [
  { id: "general", href: "/settings/", label: "settings.nav.general" },
  { id: "privacy", href: "/settings/privacy/", label: "settings.nav.privacy" },
  { id: "diagnostics", href: "/settings/diagnostics/", label: "settings.nav.diagnostics" },
];

/** Links between the settings pages. Each page says which one it is, so the current link is marked. */
export function SettingsNav({ current }: { current: SettingsPage }) {
  const t = useT();
  return (
    <nav aria-label={t("settings.nav")}>
      <ul className="subnav">
        {PAGES.map((page) => (
          <li key={page.id}>
            <Link href={page.href} className="subnav__link" aria-current={page.id === current ? "page" : undefined}>
              {t(page.label)}
            </Link>
          </li>
        ))}
      </ul>
    </nav>
  );
}
