"use client";

import { DisplaySettings } from "@/components/DisplaySettings";
import { ErrorBanner } from "@/components/ErrorBanner";
import { PrivacyStatements } from "@/components/PrivacyStatements";
import { UnavailableNote } from "@/components/UnavailableNote";
import { ProviderManager } from "@/providers/ProviderManager";
import { useT } from "@/state/PreferencesProvider";
import { LinkButton } from "@/ui/Button";
import { Panel } from "@/ui/Panel";
import { LearningSettings } from "./LearningSettings";
import { useLanguage } from "./useLanguage";

/**
 * The general settings page: provider profiles, display, language and the
 * settings that work in this build. Parts whose route the program does not
 * have (audio devices, model manager, pronunciation timing, recordings) say so.
 */
export function GeneralSettings() {
  const t = useT();
  const { setLanguage, error } = useLanguage();
  return (
    <div className="px-stack" style={{ "--gap": "var(--space-6)" } as React.CSSProperties}>
      <Panel title={t("settings.providers.title")} raised>
        <div className="px-stack">
          <p>{t("settings.providers.intro")}</p>
          <PrivacyStatements only={["privacy.key", "privacy.keysent"]} />
          <ProviderManager />
          <div>
            <LinkButton href="/onboarding/" variant="ghost">
              {t("settings.providers.setup")}
            </LinkButton>
          </div>
        </div>
      </Panel>

      <div className="px-stack">
        <DisplaySettings onLanguageChange={(language) => void setLanguage(language)} />
        {error ? (
          <>
            <p className="px-error">{t("onb.language.error")}</p>
            <ErrorBanner error={error} />
          </>
        ) : null}
      </div>

      <LearningSettings />

      <Panel title={t("settings.audio.title")}>
        <UnavailableNote feature="speech" />
      </Panel>
      <Panel title={t("settings.models.title")}>
        <UnavailableNote feature="models" />
      </Panel>
      <Panel title={t("settings.timing.title")}>
        <UnavailableNote feature="speech" />
      </Panel>
    </div>
  );
}
