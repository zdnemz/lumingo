"use client";

import { useId } from "react";
import { Sprite } from "@/sprites/Sprite";
import { useT } from "@/state/PreferencesProvider";
import { Button } from "@/ui/Button";
import { Panel } from "@/ui/Panel";

export interface MarkedError {
  id: string;
  /** The catalog name of the error type, already in the learner's language. */
  label: string;
  correction: string;
  /** The catalog explanation, already in the learner's language. */
  explanation: string;
}

export interface Segment {
  text: string;
  /** Present when these words are a mistake. */
  error?: MarkedError;
}

export interface AnnotatedTextProps {
  segments: readonly Segment[];
  selectedId: string | null;
  onSelect: (id: string | null) => void;
}

/**
 * The learner's draft with each mistake marked. A mark is a button, so the
 * keyboard reaches every one, and the mark has an icon and a label as well as
 * colour. Choosing a mark opens its correction and explanation below the text.
 */
export function AnnotatedText({ segments, selectedId, onSelect }: AnnotatedTextProps) {
  const t = useT();
  const detailId = useId();
  const errors = segments.flatMap((segment) => (segment.error ? [segment.error] : []));
  const selected = errors.find((error) => error.id === selectedId) ?? null;

  return (
    <div className="annotated">
      <p className="annotated__text" aria-label={t("write.marked")}>
        {segments.map((segment, index) => {
          const error = segment.error;
          if (!error) return <span key={index}>{segment.text}</span>;
          const number = errors.indexOf(error) + 1;
          return (
            <button
              key={index}
              type="button"
              className="annotated__mark"
              aria-expanded={selectedId === error.id}
              aria-controls={detailId}
              aria-label={`${t("write.error", { number, total: errors.length, label: error.label })}: ${segment.text}`}
              onClick={() => onSelect(selectedId === error.id ? null : error.id)}
            >
              {segment.text}
              <Sprite name="icon-cross" scale={1} className="annotated__icon" />
            </button>
          );
        })}
      </p>
      <div id={detailId} aria-live="polite">
        {selected ? (
          <Panel tone="danger" inset className="annotated__detail">
            <p className="annotated__label">{selected.label}</p>
            <p>{t("write.fix", { correction: selected.correction })}</p>
            <p className="annotated__why">
              <strong>{t("write.why")}: </strong>
              {selected.explanation}
            </p>
            <Button small onClick={() => onSelect(null)}>
              {t("write.close")}
            </Button>
          </Panel>
        ) : null}
      </div>
    </div>
  );
}
