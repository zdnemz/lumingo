"use client";

import { useT } from "@/state/PreferencesProvider";
import { Button } from "@/ui/Button";
import { Panel } from "@/ui/Panel";
import { KEY_GUIDE, KEY_GUIDE_PRESET, type ProviderPreset } from "./keyGuide";

export interface KeyGuidePanelProps {
  /** Puts the guide's values into the provider form. Without it the button is not shown. */
  onFill?: (preset: ProviderPreset) => void;
  filled?: boolean;
}

/**
 * Step by step key guide for one provider with a free tier, with that provider's
 * data-use statement (FR-A7). The name ends in "Panel" because `KeyGuide.tsx`
 * and `keyGuide.ts` differ only in case, and Windows — where CI runs — cannot
 * keep two such files apart.
 */
export function KeyGuidePanel({ onFill, filled }: KeyGuidePanelProps) {
  const t = useT();
  return (
    <Panel title={t("guide.title")} raised>
      <div className="px-stack">
        <p>{t("guide.intro")}</p>
        <ol className="guide__steps">
          <li>{t("guide.step1")}</li>
          <li>{t("guide.step2")}</li>
          <li>{t("guide.step3")}</li>
        </ol>
        <p>
          <a href={KEY_GUIDE.url} target="_blank" rel="noopener noreferrer">
            {t("guide.link")}
          </a>
        </p>

        <section aria-labelledby="guide-values" className="px-stack" style={{ "--gap": "var(--space-2)" } as React.CSSProperties}>
          <h3 id="guide-values">{t("guide.values")}</h3>
          <dl className="facts">
            <div>
              <dt>{t("provider.protocol")}</dt>
              <dd>{t("provider.protocol.openai_chat")}</dd>
            </div>
            <div>
              <dt>{t("provider.base_url")}</dt>
              <dd className="phonetic guide__value">{KEY_GUIDE.profile.base_url}</dd>
            </div>
            <div>
              <dt>{t("provider.model")}</dt>
              <dd className="phonetic guide__value">{KEY_GUIDE.profile.model}</dd>
            </div>
          </dl>
          <p className="px-hint">{t("guide.model.note")}</p>
          {onFill ? (
            <div className="px-row">
              <Button onClick={() => onFill(KEY_GUIDE_PRESET)}>{t("guide.fill")}</Button>
              {filled ? <span role="status">{t("guide.filled")}</span> : null}
            </div>
          ) : null}
        </section>

        <section aria-labelledby="guide-data" className="px-stack" style={{ "--gap": "var(--space-2)" } as React.CSSProperties}>
          <h3 id="guide-data">{t("guide.data.title")}</h3>
          <p>{t("guide.data.free")}</p>
          <p>{t("guide.data.paid")}</p>
          <p>{t("guide.data.advice")}</p>
          <p>{t("guide.data.lumingo")}</p>
          <p className="px-hint">{t("guide.data.dated", { date: KEY_GUIDE.statedOn })}</p>
        </section>
      </div>
    </Panel>
  );
}
