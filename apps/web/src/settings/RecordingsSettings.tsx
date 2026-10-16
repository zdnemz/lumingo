"use client";

import { ErrorBanner } from "@/components/ErrorBanner";
import { useT } from "@/state/PreferencesProvider";
import { Panel } from "@/ui/Panel";
import { Switch } from "@/ui/Switch";
import { useSettingsField } from "./useSettingsField";

/**
 * The opt-in for keeping recordings of the learner's voice, next to the
 * sentence that says raw audio is not kept without it. The choice is stored by
 * the program; this build has no voice loop, so nothing is recorded yet and
 * the panel says so instead of pretending otherwise.
 */
export function RecordingsSettings() {
  const t = useT();
  const field = useSettingsField();

  if (field.state.status === "loading") {
    return (
      <Panel title={t("settings.recordings.title")}>
        <p role="status">{t("state.loading")}</p>
      </Panel>
    );
  }
  if (field.state.status === "error") {
    return (
      <Panel title={t("settings.recordings.title")}>
        <ErrorBanner error={field.state.error} onRetry={field.reload} />
      </Panel>
    );
  }

  return (
    <Panel title={t("settings.recordings.title")}>
      <div className="px-stack">
        <p>{t("privacy.audio")}</p>
        <Switch
          label={t("settings.recordings.title")}
          hint={t("settings.recordings.hint")}
          checked={field.state.data.keep_recordings}
          onText={t("settings.recordings.on")}
          offText={t("settings.recordings.off")}
          disabled={field.busy}
          onChange={(keep_recordings) => void field.save({ keep_recordings })}
        />
        <p className="px-hint">{t("settings.recordings.stored")}</p>
        {field.error ? <ErrorBanner error={field.error} /> : null}
        {field.saved ? <p role="status">{t("state.saved")}</p> : null}
      </div>
    </Panel>
  );
}
