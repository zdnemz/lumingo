"use client";

import { useCallback, useMemo, useState } from "react";
import { useApi } from "@/api/ApiProvider";
import { useServerState } from "@/api/ServerState";
import { ErrorBanner } from "@/components/ErrorBanner";
import type { UnitDetail } from "@/generated/UnitDetail";
import { usePreferences, useT } from "@/state/PreferencesProvider";
import { useResource } from "@/state/useResource";
import { Switch } from "@/ui/Switch";
import { helpStartsOn } from "./model";
import { LessonPlayer } from "./LessonPlayer";
import { UnitIntro } from "./UnitIntro";
import { UnitPicker } from "./UnitPicker";

/**
 * The unit screen: the picker when no unit is named in the query string, and the
 * run of one unit when one is.
 *
 * The run is the offline path, so this screen mounts no first-run gate: the gate
 * belongs to the home page, where a learner arrives before anything is set up. A
 * learner who opened a unit wants the lesson, and the server tells the screen
 * what of it can run here.
 */
export function UnitScreen({ id }: { id: string | null }) {
  const t = useT();
  if (id === null) {
    return (
      <div className="px-stack" style={{ "--gap": "var(--space-5)" } as React.CSSProperties}>
        <h1>{t("unit.title")}</h1>
        <UnitPicker />
      </div>
    );
  }
  return <UnitRun id={id} />;
}

/** One unit: the Indonesian help toggle, what it teaches, and the run the server drives. */
function UnitRun({ id }: { id: string }) {
  const t = useT();
  const api = useApi();
  const { display } = usePreferences();
  const { snapshot } = useServerState();
  const load = useCallback((signal: AbortSignal) => api.getUnit(id, signal), [api, id]);
  const detail = useResource<UnitDetail>(load);
  const [answered, setAnswered] = useState<readonly string[]>([]);
  const [help, setHelp] = useState<boolean | null>(null);

  const onAnswered = useCallback((activityId: string) => {
    setAnswered((current) => (current.includes(activityId) ? current : [...current, activityId]));
  }, []);
  const answeredIds = useMemo(() => new Set(answered), [answered]);

  const unit = detail.state.status === "ready" ? detail.state.data.unit : null;
  // The setting arrives with the first snapshot, so the help starts off until it does.
  const mode = snapshot?.settings.l1_help_mode ?? null;
  const helpOn = help ?? (unit !== null && helpStartsOn(mode, unit.level));

  if (detail.state.status === "loading") return <p role="status">{t("state.loading")}</p>;
  if (detail.state.status === "error") return <ErrorBanner error={detail.state.error} onRetry={detail.reload} />;
  if (unit === null) return null;

  const title = display.language === "id" ? (unit.title.id ?? unit.title.en) : unit.title.en;

  return (
    <div className="px-stack" style={{ "--gap": "var(--space-5)" } as React.CSSProperties}>
      <div className="px-stack" style={{ "--gap": "var(--space-3)" } as React.CSSProperties}>
        <h1>{title}</h1>
        <Switch
          checked={helpOn}
          onChange={setHelp}
          label={t("unit.help.toggle")}
          onText={t("unit.help.on")}
          offText={t("unit.help.off")}
          hint={t("settings.l1help.hint")}
        />
      </div>
      <UnitIntro unit={unit} answeredIds={answeredIds} help={helpOn} />
      <LessonPlayer unit={unit} help={helpOn} onAnswered={onAnswered} />
    </div>
  );
}
