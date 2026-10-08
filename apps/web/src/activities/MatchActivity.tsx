"use client";

import { useId } from "react";
import { useT } from "@/state/PreferencesProvider";

export interface MatchActivityProps {
  /** The items on the left, in order. */
  left: readonly string[];
  /** The possible matches, in the shuffled order the learner sees. */
  right: readonly string[];
  /** For each left item, the chosen right index, or null. */
  chosen: readonly (number | null)[];
  onChoose: (leftIndex: number, rightIndex: number | null) => void;
  locked?: boolean;
}

/**
 * Pair each item with its match using a list of choices per row, which works
 * with a keyboard, a screen reader and a touch screen without any dragging.
 */
export function MatchActivity({ left, right, chosen, onChoose, locked }: MatchActivityProps) {
  const t = useT();
  const name = useId();
  return (
    <ul className="match">
      {left.map((item, index) => {
        const id = `${name}-${index}`;
        return (
          <li key={index} className="match__row">
            <label htmlFor={id} className="match__item">
              {item}
            </label>
            <select
              id={id}
              aria-label={t("activity.match.pick", { item })}
              className="px-field match__select"
              disabled={locked}
              value={chosen[index] ?? ""}
              onChange={(event) => onChoose(index, event.target.value === "" ? null : Number(event.target.value))}
            >
              <option value="">{t("activity.match.none")}</option>
              {right.map((option, optionIndex) => (
                <option key={optionIndex} value={optionIndex}>
                  {option}
                </option>
              ))}
            </select>
          </li>
        );
      })}
    </ul>
  );
}
