"use client";

import { useId } from "react";
import { useT } from "@/state/PreferencesProvider";

export interface TextAnswerProps {
  value: string;
  onChange: (value: string) => void;
  /** One line for a short answer, several for writing. */
  rows?: number;
  /** A word range to aim for, for writing tasks. */
  words?: { min: number; max: number };
  locked?: boolean;
}

export function countWords(text: string): number {
  const trimmed = text.trim();
  return trimmed === "" ? 0 : trimmed.split(/\s+/).length;
}

/** A text box for dictation, error correction and writing, with a word count for tasks that have a range. */
export function TextAnswer({ value, onChange, rows = 1, words, locked }: TextAnswerProps) {
  const t = useT();
  const id = useId();
  const hintId = useId();
  const count = countWords(value);
  const hint = words
    ? t("activity.text.range", { count, min: words.min, max: words.max })
    : rows > 1
      ? t("activity.text.count", { count })
      : null;
  return (
    <div className="textanswer">
      <label className="px-label" htmlFor={id}>
        {t("activity.text.label")}
      </label>
      {rows > 1 ? (
        <textarea
          id={id}
          className="px-field"
          rows={rows}
          value={value}
          disabled={locked}
          aria-describedby={hint ? hintId : undefined}
          onChange={(event) => onChange(event.target.value)}
        />
      ) : (
        <input
          id={id}
          className="px-field"
          value={value}
          disabled={locked}
          autoComplete="off"
          spellCheck={false}
          onChange={(event) => onChange(event.target.value)}
        />
      )}
      {hint ? (
        <p id={hintId} className="px-hint" aria-live="off">
          {hint}
        </p>
      ) : null}
    </div>
  );
}
