"use client";

import type { Unit } from "@/generated/Unit";
import { usePreferences, useT } from "@/state/PreferencesProvider";
import { Panel } from "@/ui/Panel";
import { localized } from "./model";

export interface CheckpointView {
  rows: readonly { activity_id: string; answered: boolean; score: number | null }[];
  mean: number;
  pass_mark: number;
  passed: boolean;
  provisional: boolean;
  answered: number;
}

/**
 * The end of the run, exactly as `POST /api/sessions/{id}/stop` reported it: the
 * rows the run answered, the mean of the scored checkpoint items and the pass
 * mark. Nothing is recomputed here — a row the server left unanswered stays
 * "not answered", and a mean of zero is shown as zero.
 *
 * The activity each row names is written out with the unit's own instruction,
 * so a row reads as an activity rather than as an id.
 */
export function CheckpointPanel({ result, unit }: { result: CheckpointView; unit: Unit }) {
  const t = useT();
  const { display } = usePreferences();
  const instruction = (id: string): string => {
    const activity = unit.activities.find((entry) => entry.id === id);
    return activity ? localized(activity.instructions, display.language) : id;
  };
  const percent = (value: number): number => Math.round(value * 100);
  return (
    <Panel title={t("unit.checkpoint.title")} raised>
      <div className="px-stack" style={{ "--gap": "var(--space-3)" } as React.CSSProperties}>
        <p className="px-hint">{t("unit.checkpoint.intro")}</p>
        <p>
          <strong>{t("unit.checkpoint.mean", { percent: percent(result.mean) })}</strong>{" "}
          <span>{t("unit.checkpoint.passmark", { percent: percent(result.pass_mark) })}</span>
        </p>
        <p>{t("unit.checkpoint.answered", { answered: result.answered, total: result.rows.length })}</p>
        <p>{result.passed ? t("unit.checkpoint.passed") : t("unit.checkpoint.notpassed")}</p>
        {result.provisional ? <p className="px-hint">{t("unit.checkpoint.provisional")}</p> : null}
        {result.rows.length === 0 ? (
          <p>{t("unit.checkpoint.none")}</p>
        ) : (
          <div className="table-wrap">
            <table className="table">
              <thead>
                <tr>
                  <th scope="col">{t("unit.checkpoint.activity")}</th>
                  <th scope="col">{t("unit.checkpoint.result")}</th>
                </tr>
              </thead>
              <tbody>
                {result.rows.map((row) => (
                  <tr key={row.activity_id}>
                    <th scope="row">{instruction(row.activity_id)}</th>
                    <td>{row.score === null ? t("unit.checkpoint.unanswered") : t("unit.checkpoint.percent", { percent: percent(row.score) })}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>
    </Panel>
  );
}
