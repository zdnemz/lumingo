"use client";

import { useT } from "@/state/PreferencesProvider";
import { Button } from "@/ui/Button";

export interface ReorderActivityProps {
  /** Every word, in the scrambled order the learner sees. */
  tokens: readonly string[];
  /** Positions in `tokens` the learner has placed, in order. */
  placed: readonly number[];
  onPlace: (tokenIndex: number) => void;
  onRemove: (position: number) => void;
  onReset: () => void;
  locked?: boolean;
}

/**
 * Build a sentence by choosing words in order. Every word is a button, so the
 * activity works with a keyboard and a screen reader, and there is no dragging.
 */
export function ReorderActivity({ tokens, placed, onPlace, onRemove, onReset, locked }: ReorderActivityProps) {
  const t = useT();
  const used = new Set(placed);
  return (
    <div className="reorder">
      <section aria-label={t("activity.reorder.answer")}>
        <h3 className="reorder__title">{t("activity.reorder.answer")}</h3>
        <ol className="reorder__answer px-panel px-panel--inset">
          {placed.length === 0 ? <li className="px-hint">{t("activity.reorder.empty")}</li> : null}
          {placed.map((tokenIndex, position) => (
            <li key={`${tokenIndex}-${position}`}>
              <Button
                small
                disabled={locked}
                onClick={() => onRemove(position)}
                aria-label={t("activity.reorder.remove", { word: tokens[tokenIndex] ?? "" })}
              >
                {tokens[tokenIndex]}
              </Button>
            </li>
          ))}
        </ol>
      </section>
      <section aria-label={t("activity.reorder.pool")}>
        <h3 className="reorder__title">{t("activity.reorder.pool")}</h3>
        <ul className="reorder__pool">
          {tokens.map((token, index) => (
            <li key={index}>
              <Button
                small
                variant="ghost"
                disabled={locked || used.has(index)}
                onClick={() => onPlace(index)}
                aria-label={t("activity.reorder.add", { word: token })}
              >
                {token}
              </Button>
            </li>
          ))}
        </ul>
      </section>
      <Button small variant="ghost" disabled={locked || placed.length === 0} onClick={onReset}>
        {t("activity.reorder.reset")}
      </Button>
    </div>
  );
}
