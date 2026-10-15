// Pure rules for the reading screen. They read what the server sent — the
// outcome of a generation, the questions and the score — and never write a
// text, invent a gloss, or decide a score of their own.

import type { Level } from "@/generated/Level";
import type { QuestionResultView } from "@/generated/QuestionResultView";
import type { ReadingOutcomeView } from "@/generated/ReadingOutcomeView";
import type { ReadingTarget } from "@/generated/ReadingTarget";
import type { ReadingTextView } from "@/generated/ReadingTextView";
import type { Language } from "@/state/preferences";

/**
 * The levels of the generated `Level` union, in learning order. The record makes
 * a level added to the union a compile error here, so the picker can never fall
 * behind it, and a level dropped from the union leaves a key the compiler
 * refuses. The labels are the scale names themselves: they are data, not
 * sentences, so they are not in the dictionaries.
 */
const ORDER: Record<Level, true> = { A1: true, A2: true, B1: true, B2: true, C1: true, C2: true };

export const LEVELS: readonly Level[] = Object.keys(ORDER) as Level[];

/** The level a session reads at until the learner picks another one. */
export const DEFAULT_LEVEL: Level = "A1";

/**
 * One text the screen can show: a text the provider wrote, or an authored set
 * of a unit offered in its place. The target is what the answers belong to, so
 * the screen never has to guess which route the score came from.
 */
export type OfferedReading = { key: string; text: ReadingTextView } & (
  | { kind: "generated"; contentId: number; regenerated: boolean; vocabularyChecked: boolean }
  | { kind: "authored"; unitId: string; activityId: string }
);

/** Which text the answers are for, as `POST /api/reading/{session}/answers` wants it. */
export function targetOf(reading: OfferedReading): ReadingTarget {
  return reading.kind === "generated"
    ? { kind: "generated", content_id: reading.contentId }
    : { kind: "authored", unit_id: reading.unitId, activity_id: reading.activityId };
}

/** Every text the outcome offers, in the order the server sent them. */
export function offeredReadings(outcome: ReadingOutcomeView): OfferedReading[] {
  if (outcome.kind === "generated") {
    return [
      {
        kind: "generated",
        key: `generated-${outcome.content_id}`,
        contentId: outcome.content_id,
        text: outcome.text,
        regenerated: outcome.regenerated,
        vocabularyChecked: outcome.vocabulary_checked,
      },
    ];
  }
  return outcome.sets.map((set) => ({
    kind: "authored",
    key: `${set.unit_id}/${set.activity_id}`,
    unitId: set.unit_id,
    activityId: set.activity_id,
    text: set.text,
  }));
}

/**
 * The answer list the engine accepts: exactly one entry per question, `null` for
 * one left open. A list of another length is refused, so it is built from the
 * question count and never from the picks.
 */
export function answersOf(questions: number, picks: readonly (number | null)[]): (number | null)[] {
  return Array.from({ length: questions }, (_, index) => picks[index] ?? null);
}

/**
 * The explanation in the learner's own language. The other language is used only
 * when that side is empty, so a half-written explanation is still shown.
 */
export function explanationText(result: QuestionResultView, language: Language): string {
  const first = language === "id" ? result.explanation_l1 : result.explanation_en;
  const second = language === "id" ? result.explanation_en : result.explanation_l1;
  return first.trim() !== "" ? first : second;
}
