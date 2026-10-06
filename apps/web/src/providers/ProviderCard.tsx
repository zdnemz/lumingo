"use client";

import type { ReactNode } from "react";
import type { ProviderInfo } from "@/generated/ProviderInfo";
import { useT, usePreferences } from "@/state/PreferencesProvider";
import { Badge } from "@/ui/Badge";
import { Panel } from "@/ui/Panel";

/** Formats an RFC 3339 time for the learner. Falls back to the text the program sent. */
export function useFormatTime(): (value: string) => string {
  const { display } = usePreferences();
  return (value) => {
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) return value;
    return new Intl.DateTimeFormat(display.language, { dateStyle: "medium", timeStyle: "short" }).format(date);
  };
}

export interface ProviderCardProps {
  provider: ProviderInfo;
  /** Buttons for this profile. */
  actions?: ReactNode;
  /** Anything that belongs under the facts, such as a test result. */
  children?: ReactNode;
}

/**
 * One provider profile as the program lets the page see it. The key itself is
 * not in the data: the card shows only whether one is saved and where it ends.
 */
export function ProviderCard({ provider, actions, children }: ProviderCardProps) {
  const t = useT();
  const formatTime = useFormatTime();
  const keyLine = !provider.has_key
    ? t("provider.key.none")
    : provider.key_last4
      ? t("provider.key.has", { last4: `••••${provider.key_last4}` })
      : t("provider.key.has.short");
  return (
    <Panel as="article" raised tone={provider.is_active ? "success" : undefined} aria-label={provider.name}>
      <div className="px-stack" style={{ "--gap": "var(--space-3)" } as React.CSSProperties}>
        <div className="px-row">
          <h3>{provider.name}</h3>
          {provider.is_active ? <Badge tone="success">{t("provider.active")}</Badge> : null}
          <Badge tone="info">{provider.source === "env" ? t("provider.source.env") : t("provider.source.file")}</Badge>
        </div>
        <dl className="facts">
          <div>
            <dt>{t("provider.protocol")}</dt>
            <dd>{t(`provider.protocol.${provider.protocol}`)}</dd>
          </div>
          <div>
            <dt>{t("provider.base_url")}</dt>
            <dd className="phonetic facts__wrap">{provider.base_url}</dd>
          </div>
          <div>
            <dt>{t("provider.model")}</dt>
            <dd className="phonetic facts__wrap">{provider.model}</dd>
          </div>
          <div>
            <dt>{t("provider.key")}</dt>
            <dd>{keyLine}</dd>
          </div>
        </dl>
        <p className="px-hint">
          {provider.probed_at ? t("provider.tested.at", { time: formatTime(provider.probed_at) }) : t("provider.tested.never")}
        </p>
        {provider.source === "env" ? <p className="px-hint">{t("provider.env.note")}</p> : null}
        {children}
        {actions ? <div className="px-row">{actions}</div> : null}
      </div>
    </Panel>
  );
}
