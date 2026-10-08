"use client";

import { Fragment, useId } from "react";
import { useT } from "@/state/PreferencesProvider";

export interface GapFillActivityProps {
  /** The sentence with `___` where each gap is. */
  text: string;
  answers: readonly string[];
  onChange: (index: number, value: string) => void;
  locked?: boolean;
}

/** The marker the unit files use for a gap. */
export const GAP_MARK = "___";

/** Counts the gaps in a text. */
export function gapCount(text: string): number {
  return text.split(GAP_MARK).length - 1;
}

export function GapFillActivity({ text, answers, onChange, locked }: GapFillActivityProps) {
  const t = useT();
  const name = useId();
  const parts = text.split(GAP_MARK);
  return (
    <p className="gapfill">
      {parts.map((part, index) => (
        <Fragment key={index}>
          {part}
          {index < parts.length - 1 ? (
            <input
              className="px-field gapfill__input"
              aria-label={t("activity.gap", { number: index + 1 })}
              id={`${name}-${index}`}
              value={answers[index] ?? ""}
              disabled={locked}
              autoComplete="off"
              autoCapitalize="none"
              spellCheck={false}
              onChange={(event) => onChange(index, event.target.value)}
            />
          ) : null}
        </Fragment>
      ))}
    </p>
  );
}
