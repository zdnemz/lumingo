"use client";

import { Fragment, useId } from "react";
import { useT } from "@/state/PreferencesProvider";
import { Button } from "@/ui/Button";
import { Panel } from "@/ui/Panel";

export interface GlossEntry {
  /** The word as it appears in the text, compared without regard to case. */
  word: string;
  /** The meaning in the learner's language. */
  gloss: string;
}

export interface GlossedTextProps {
  text: string;
  glossary: readonly GlossEntry[];
  /** The word whose meaning is open, or null. */
  openWord: string | null;
  onOpen: (word: string | null) => void;
  /** Index of the sentence being read aloud, if any, for highlighting. */
  currentSentence?: number;
}

/** Splits text into sentences while keeping their original spacing. */
export function splitSentences(text: string): string[] {
  return text.match(/[^.!?]+(?:[.!?]+["')\]]*|$)\s*/g)?.filter((s) => s.trim() !== "") ?? [];
}

const WORD = /([A-Za-z][A-Za-z'-]*)/g;

/**
 * A reading passage in which glossary words are buttons. The meaning opens under
 * the text. The sentence read aloud is highlighted with a marker, not by colour
 * alone, and the page never scrolls on its own.
 */
export function GlossedText({ text, glossary, openWord, onOpen, currentSentence }: GlossedTextProps) {
  const t = useT();
  const detailId = useId();
  const lookup = new Map(glossary.map((entry) => [entry.word.toLowerCase(), entry]));
  const open = openWord ? lookup.get(openWord.toLowerCase()) : undefined;
  const sentences = splitSentences(text);

  return (
    <div className="gloss">
      <p className="gloss__text" aria-label={t("read.text")}>
        {sentences.map((sentence, sentenceIndex) => (
          <span key={sentenceIndex} className="gloss__sentence" data-current={currentSentence === sentenceIndex || undefined}>
            {sentence.split(WORD).map((piece, pieceIndex) => {
              const entry = pieceIndex % 2 === 1 ? lookup.get(piece.toLowerCase()) : undefined;
              if (!entry) return <Fragment key={pieceIndex}>{piece}</Fragment>;
              return (
                <button
                  key={pieceIndex}
                  type="button"
                  className="gloss__word"
                  aria-label={t("read.word", { word: piece })}
                  aria-expanded={openWord?.toLowerCase() === piece.toLowerCase()}
                  aria-controls={detailId}
                  onClick={() => onOpen(openWord?.toLowerCase() === piece.toLowerCase() ? null : piece)}
                >
                  {piece}
                </button>
              );
            })}
          </span>
        ))}
      </p>
      <div id={detailId} aria-live="polite">
        {open ? (
          <Panel tone="reading" inset className="gloss__detail">
            <p>{t("read.gloss", { word: open.word, gloss: open.gloss })}</p>
            <Button small onClick={() => onOpen(null)}>
              {t("read.gloss.close")}
            </Button>
          </Panel>
        ) : null}
      </div>
    </div>
  );
}
