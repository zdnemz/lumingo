"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import type { ReactNode } from "react";
import { Mascot } from "@/sprites/Mascot";
import { Sprite } from "@/sprites/Sprite";
import type { SpriteName } from "@/sprites/data";
import { useT } from "@/state/PreferencesProvider";
import type { MessageKey } from "@/i18n";

interface NavItem {
  href: string;
  label: MessageKey;
  icon: SpriteName;
}

// A screen appears here when it exists. Links to screens that are not built yet would be dead ends.
const NAV: readonly NavItem[] = [
  { href: "/", label: "nav.home", icon: "icon-map" },
  { href: "/settings/", label: "nav.settings", icon: "icon-gear" },
];

function isCurrent(pathname: string | null, href: string): boolean {
  if (pathname === null) return false;
  const clean = (value: string) => (value.length > 1 ? value.replace(/\/$/, "") : value);
  return clean(pathname) === clean(href);
}

export function AppShell({ children }: { children: ReactNode }) {
  const t = useT();
  const pathname = usePathname();
  return (
    <div className="shell px-sky px-sky--twinkle">
      <a className="skip-link" href="#main">
        {t("nav.skip")}
      </a>
      <header className="shell__bar">
        <div className="px-container shell__bar-inner">
          <Link href="/" className="shell__brand">
            <Mascot mood="idle" scale={3} />
            <span className="shell__name">{t("app.name")}</span>
          </Link>
          <nav aria-label={t("nav.main")}>
            <ul className="shell__nav">
              {NAV.map((item) => (
                <li key={item.href}>
                  <Link
                    href={item.href}
                    className="shell__link"
                    aria-current={isCurrent(pathname, item.href) ? "page" : undefined}
                  >
                    <Sprite name={item.icon} scale={2} />
                    <span>{t(item.label)}</span>
                  </Link>
                </li>
              ))}
            </ul>
          </nav>
        </div>
      </header>
      <main id="main" className="px-container shell__main" tabIndex={-1}>
        {children}
      </main>
    </div>
  );
}
