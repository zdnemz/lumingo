"use client";

import type { ActivityOutcomeView } from "@/generated/ActivityOutcomeView";
import type { ActivityResultView } from "@/generated/ActivityResultView";
import type { Localized } from "@/generated/Localized";
import type { ActivityResult } from "@/activities/ActivityFrame";

/** The accepted answer to show after a wrong or partial answer: the first item that was not right. */
function acceptedAnswer(outcome: ActivityOutcomeView): string | undefined {
  if (outcome.kind !== "deterministic") return undefined;
  return outcome.feedback.items.find((item) => item.outcome !== "correct")?.expected;
}

/** The explanation the server sent with the result, in the learner's language when it has one. */
function explanation(outcome: ActivityOutcomeView, language: "en" | "id"): string | undefined {
  const pick = (value: Localized | null): string | undefined =>
    value === null ? undefined : language === "id" ? (value.id ?? value.en) : value.en;
  switch (outcome.kind) {
    case "deterministic":
      return pick(outcome.feedback.explanation);
    case "rubric":
      return language === "id" && outcome.feedback.feedback_l1 !== "" ? outcome.feedback.feedback_l1 : outcome.feedback.feedback_en;
    default:
      return undefined;
  }
}

/**
 * What `ActivityFrame` needs from a server result, and nothing more: the score
 * is the server's, the accepted answer is the server's own `expected` text. A
 * result with no score (`queued`, a pronunciation run) leaves the score out, so
 * the frame says the answer waits for the provider.
 *
 * An `unscored` result is not passed through here: it has no score and is not
 * waiting for anyone, so the walk shows the server's reason in its own panel.
 */
export function frameResult(result: ActivityResultView, language: "en" | "id"): ActivityResult {
  return {
    score: result.score,
    explanation: explanation(result.outcome, language),
    correctAnswer: acceptedAnswer(result.outcome),
  };
}
