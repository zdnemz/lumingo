"use client";

import { useCallback, useState, type FormEvent } from "react";
import { useApi } from "@/api/ApiProvider";
import { ErrorBanner } from "@/components/ErrorBanner";
import type { L1HelpMode } from "@/generated/L1HelpMode";
import type { Settings } from "@/generated/Settings";
import { useT } from "@/state/PreferencesProvider";
import { useAction, useResource } from "@/state/useResource";
import { Button } from "@/ui/Button";
import { Panel } from "@/ui/Panel";
import { Segmented } from "@/ui/Segmented";
import { TextField } from "@/ui/TextField";

const L1_MODES: readonly L1HelpMode[] = ["auto", "on", "off"];
const MAX_NAME_CHARS = 64;

function validName(value: string): boolean {
  const name = value.trim();
  // Control characters are refused by the program too; checking here saves a round trip.
  return name.length > 0 && [...name].length <= MAX_NAME_CHARS && !/[\u0000-\u001f\u007f]/.test(name);
}

/**
 * The settings that work in this build: what the tutor calls the learner and
 * whether Indonesian help is on. The program takes every setting in one PUT,
 * so each change sends the whole current set.
 */
export function LearningSettings() {
  const t = useT();
  const api = useApi();
  const load = useCallback((signal: AbortSignal) => api.getSettings(signal), [api]);
  const settings = useResource<Settings>(load);
  const action = useAction();
  const [name, setName] = useState<string | null>(null);
  const [attempted, setAttempted] = useState(false);
  const [saved, setSaved] = useState(false);

  if (settings.state.status === "loading") return <p role="status">{t("state.loading")}</p>;
  if (settings.state.status === "error") return <ErrorBanner error={settings.state.error} onRetry={settings.reload} />;
  const current = settings.state.data;
  const shownName = name ?? current.display_name;

  const save = async (next: Settings) => {
    setSaved(false);
    const stored = await action.run(() => api.updateSettings(next));
    if (stored) {
      settings.replace(stored);
      setName(null);
      setAttempted(false);
      setSaved(true);
    }
  };

  const submitName = (event: FormEvent) => {
    event.preventDefault();
    setAttempted(true);
    if (!validName(shownName) || action.busy) return;
    void save({ ...current, display_name: shownName.trim() });
  };

  return (
    <Panel title={t("settings.learning.title")} raised>
      <div className="px-stack">
        <form className="px-stack" onSubmit={submitName} noValidate>
          <TextField
            label={t("settings.name")}
            value={shownName}
            autoComplete="off"
            error={attempted && !validName(shownName) ? t("settings.name.error") : undefined}
            onChange={(value) => {
              setName(value);
              setSaved(false);
            }}
          />
          <div className="px-row">
            <Button type="submit" busy={action.busy}>
              {t("settings.name.save")}
            </Button>
          </div>
        </form>
        <Segmented
          legend={t("settings.l1help")}
          hint={t("settings.l1help.hint")}
          value={current.l1_help_mode}
          onChange={(l1_help_mode) => void save({ ...current, l1_help_mode })}
          options={L1_MODES.map((value) => ({ value, label: t(`settings.l1help.${value}`) }))}
        />
        {action.error ? <ErrorBanner error={action.error} /> : null}
        {saved ? <p role="status">{t("state.saved")}</p> : null}
      </div>
    </Panel>
  );
}
