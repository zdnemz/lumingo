"use client";

import { useId } from "react";
import { useT } from "@/state/PreferencesProvider";

export interface McqActivityProps {
  stem: string;
  options: readonly string[];
  /** A passage to read first, for reading questions. */
  passage?: string;
  selected: number | null;
  onSelect: (index: number) => void;
  /** After checking: which option was right, and the choices lock. */
  correctIndex?: number;
  locked?: boolean;
}

/** One question, several answers, one choice: a radio group drawn as large buttons. */
export function McqActivity({ stem, options, passage, selected, onSelect, correctIndex, locked }: McqActivityProps) {
  const t = useT();
  const name = useId();
  return (
    <fieldset className="mcq" disabled={locked}>
      {passage ? <p className="mcq__passage px-panel px-panel--inset">{passage}</p> : null}
      <legend className="mcq__stem">{stem}</legend>
      <span className="sr-only">{t("activity.pick-one")}</span>
      <ul className="mcq__options">
        {options.map((option, index) => {
          const id = `${name}-${index}`;
          const state =
            correctIndex === undefined
              ? undefined
              : index === correctIndex
                ? "right"
                : index === selected
                  ? "wrong"
                  : undefined;
          return (
            <li key={index} className="px-segmented__option mcq__option" data-state={state}>
              <input
                id={id}
                className="px-segmented__input"
                type="radio"
                name={name}
                checked={selected === index}
                onChange={() => onSelect(index)}
              />
              <label htmlFor={id} className="px-segmented__item mcq__label">
                {option}
              </label>
            </li>
          );
        })}
      </ul>
    </fieldset>
  );
}
