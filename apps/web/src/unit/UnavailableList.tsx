"use client";

import type { ActivityUnavailable } from "@/generated/ActivityUnavailable";
import type { Unit } from "@/generated/Unit";
import { usePreferences, useT } from "@/state/PreferencesProvider";
import { Panel } from "@/ui/Panel";
import { localized } from "./model";

/**
 * The activities of the run that this program cannot do, each with the server's
 * own sentence. There is no control for them: the program said what it lacks,
 * and the screen repeats that rather than offering a button that would fail.
 */
export function UnavailableList({ unavailable, unit }: { unavailable: readonly ActivityUnavailable[]; unit: Unit }) {
  const t = useT();
  const { display } = usePreferences();
  if (unavailable.length === 0) return null;
  return (
    <Panel title={t("unit.unavailable.title")} tone="warning" raised>
      <p className="px-hint">{t("unit.unavailable.intro")}</p>
      <ul className="unit-unavailable">
        {unavailable.map((entry) => {
          const activity = unit.activities.find((candidate) => candidate.id === entry.activity_id);
          return (
            <li key={entry.activity_id} className="unavailable" role="note">
              <p className="unavailable__title">
                <strong>{activity ? localized(activity.instructions, display.language) : entry.activity_id}</strong>
              </p>
              <p className="px-hint">{entry.reason}</p>
            </li>
          );
        })}
      </ul>
    </Panel>
  );
}
