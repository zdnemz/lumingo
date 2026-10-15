"use client";

import { useCallback, useMemo } from "react";
import { useApi } from "@/api/ApiProvider";
import { ErrorBanner } from "@/components/ErrorBanner";
import type { ProgressOverview } from "@/generated/ProgressOverview";
import type { UnitIssue } from "@/generated/UnitIssue";
import type { UnitList } from "@/generated/UnitList";
import { QuestMap } from "@/learn/QuestMap";
import { usePreferences, useT } from "@/state/PreferencesProvider";
import { useResource } from "@/state/useResource";
import { Panel } from "@/ui/Panel";
import { questRegions } from "./model";

/**
 * The picker: every unit the program indexed, drawn with the quest map.
 *
 * A node's state is not the unit list's to say, and it is not guessed here
 * either: the map is drawn only once both `GET /api/units` (the units) and
 * `GET /api/progress` (where each unit stands for this learner) have answered.
 * See `questRegions` for the mapping and why it is the simplest honest one.
 */
export function UnitPicker() {
  const t = useT();
  const api = useApi();
  const { display } = usePreferences();
  const loadUnits = useCallback((signal: AbortSignal) => api.listUnits(signal), [api]);
  const loadProgress = useCallback((signal: AbortSignal) => api.getProgress(signal), [api]);
  const units = useResource<UnitList>(loadUnits);
  const progress = useResource<ProgressOverview>(loadProgress);
  const language = display.language;

  const list = units.state.status === "ready" ? units.state.data : null;
  const overview = progress.state.status === "ready" ? progress.state.data : null;
  const regions = useMemo(
    () => (list !== null && overview !== null ? questRegions(list.units, overview, language) : null),
    [list, overview, language],
  );

  const reload = useCallback(() => {
    units.reload();
    progress.reload();
  }, [units, progress]);

  const loading = units.state.status === "loading" || progress.state.status === "loading";

  return (
    <div className="px-stack" style={{ "--gap": "var(--space-5)" } as React.CSSProperties}>
      <p className="px-hint">{t("unit.intro")}</p>

      {loading && regions === null ? <p role="status">{t("state.loading")}</p> : null}
      {units.state.status === "error" ? <ErrorBanner error={units.state.error} onRetry={reload} /> : null}
      {progress.state.status === "error" ? <ErrorBanner error={progress.state.error} onRetry={reload} /> : null}

      {regions !== null && list !== null ? (
        <>
          {list.units.length === 0 ? <p>{t("unit.empty")}</p> : <QuestMap regions={regions} />}
          {list.issues.length > 0 ? (
            <Panel tone="warning" role="note">
              <p>{t("unit.issues", { count: list.issues.length })}</p>
              <ul className="unit-issues">
                {list.issues.map((issue: UnitIssue) => (
                  <li key={issue.file} className="px-hint">
                    {issue.file}: {issue.message}
                  </li>
                ))}
              </ul>
            </Panel>
          ) : null}
        </>
      ) : null}
    </div>
  );
}
