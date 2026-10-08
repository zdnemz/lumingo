"use client";

import { Sprite } from "@/sprites/Sprite";
import type { SpriteName } from "@/sprites/data";
import { useT } from "@/state/PreferencesProvider";
import { Badge } from "@/ui/Badge";

export type PhonemeTone = "good" | "close" | "off";

export interface PhonemeResult {
  /** The expected sound, written with IPA symbols. */
  symbol: string;
  tone: PhonemeTone;
  /** The closest sound the engine heard, when it differs. */
  heard?: string;
}

export interface WordResult {
  text: string;
  phonemes: readonly PhonemeResult[];
}

const MARKS: Record<PhonemeTone, SpriteName> = {
  good: "icon-check",
  close: "icon-star",
  off: "icon-cross",
};

/**
 * Scored words and the sounds inside them. Each chip carries an icon and a text
 * label next to the colour, so the result never depends on colour alone. Every
 * view of these results says that the feedback is experimental.
 */
export function PhonemeChips({ words }: { words: readonly WordResult[] }) {
  const t = useT();
  return (
    <div className="pron">
      <Badge tone="info">{t("pron.experimental")}</Badge>
      <ul className="pron__words">
        {words.map((word, wordIndex) => (
          <li key={`${word.text}-${wordIndex}`} className="pron__word">
            <p className="pron__text">{word.text}</p>
            <ul className="pron__chips" aria-label={t("pron.word", { word: word.text })}>
              {word.phonemes.map((phoneme, index) => (
                <li key={`${phoneme.symbol}-${index}`}>
                  <span className="px-chip" data-tone={phoneme.tone}>
                    <Sprite name={MARKS[phoneme.tone]} scale={2} />
                    <span className="phonetic">{phoneme.symbol}</span>
                    <span className="sr-only">
                      {t(`pron.tone.${phoneme.tone}`)}
                      {phoneme.heard ? `. ${t("pron.heard", { sound: phoneme.heard })}` : ""}
                    </span>
                    {phoneme.heard ? (
                      <span className="pron__heard phonetic" aria-hidden="true">
                        {phoneme.heard}
                      </span>
                    ) : null}
                  </span>
                </li>
              ))}
            </ul>
          </li>
        ))}
      </ul>
    </div>
  );
}
