"use client";

import type { AdaptiveTiming } from "@/generated/AdaptiveTiming";
import { useT } from "@/state/PreferencesProvider";
import { ErrorBanner } from "@/components/ErrorBanner";
import { Panel } from "@/ui/Panel";
import { Segmented } from "@/ui/Segmented";
import { useSettingsField } from "./useSettingsField";

const TIMINGS: readonly AdaptiveTiming[] = ["auto", "always_wait", "never_wait"];
const LABELS: Record<AdaptiveTiming, "settings.timing.auto" | "settings.timing.wait" | "settings.timing.never"> = {
  auto: "settings.timing.auto",
  always_wait: "settings.timing.wait",
  never_wait: "settings.timing.never",
};

/**
 * Whether the tutor's reply waits for the pronunciation analysis. The choice is
 * stored by the program, but this build has no speech engines, so the panel
 * says plainly that nothing changes yet: the control is honest rather than
 * decorative.
 */
export function TimingSettings() {
  const t = useT();
  const field = useSettingsField();

  if (field.state.status === "loading") {
    return (
      <Panel title={t("settings.timing.title")}>
        <p role="status">{t("state.loading")}</p>
      </Panel>
    );
  }
  if (field.state.status === "error") {
    return (
      <Panel title={t("settings.timing.title")}>
        <ErrorBanner error={field.state.error} onRetry={field.reload} />
      </Panel>
    );
  }

  return (
    <Panel title={t("settings.timing.title")}>
      <div className="px-stack">
        <Segmented
          legend={t("settings.timing.title")}
          hint={t("settings.timing.hint")}
          value={field.state.data.adaptive_timing}
          onChange={(adaptive_timing) => {
            if (!field.busy) void field.save({ adaptive_timing });
          }}
          options={TIMINGS.map((value) => ({ value, label: t(LABELS[value]) }))}
        />
        <p className="px-hint">{t("settings.timing.effect")}</p>
        {field.error ? <ErrorBanner error={field.error} /> : null}
        {field.saved ? <p role="status">{t("state.saved")}</p> : null}
      </div>
    </Panel>
  );
}
