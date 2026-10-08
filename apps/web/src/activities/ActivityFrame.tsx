"use client";

import type { ReactNode } from "react";
import { Mascot } from "@/sprites/Mascot";
import { useT } from "@/state/PreferencesProvider";
import { Button } from "@/ui/Button";
import { Panel, type PanelTone } from "@/ui/Panel";
import { Badge } from "@/ui/Badge";

export interface ActivityResult {
  /** 0 to 1. `null` while the answer waits for a provider to score it. */
  score: number | null;
  explanation?: string;
  /** What the accepted answer was, shown after a wrong or partial answer. */
  correctAnswer?: string;
  /** Sparks earned for finishing this activity. */
  sparks?: number;
}

export interface ActivityFrameProps {
  skill: PanelTone;
  instructions: string;
  /** Help in the learner's first language, shown when the help setting is on. */
  help?: string;
  result?: ActivityResult;
  /** The answer is complete enough to check. */
  canSubmit: boolean;
  busy?: boolean;
  onSubmit: () => void;
  onNext?: () => void;
  children: ReactNode;
}

function verdict(score: number | null): "pending" | "correct" | "almost" | "wrong" {
  if (score === null) return "pending";
  if (score >= 0.7) return "correct";
  if (score >= 0.4) return "almost";
  return "wrong";
}

/**
 * The frame every activity sits in: the instruction, the activity itself, a
 * Check button, and the feedback after it. Feedback uses words and Lumi's face
 * as well as colour, and it is announced to screen readers.
 */
export function ActivityFrame({
  skill,
  instructions,
  help,
  result,
  canSubmit,
  busy,
  onSubmit,
  onNext,
  children,
}: ActivityFrameProps) {
  const t = useT();
  const state = result ? verdict(result.score) : null;
  const mood = state === "correct" ? "happy" : state === "wrong" ? "sad" : state === "almost" ? "think" : "idle";
  return (
    <Panel as="article" tone={skill} raised className="activity">
      <p className="activity__instructions">{instructions}</p>
      {help ? (
        <p className="activity__help">
          <Badge>{t("activity.help")}</Badge> <span lang="id">{help}</span>
        </p>
      ) : null}
      <div className="activity__body">{children}</div>
      <div className="activity__actions">
        {result ? (
          onNext ? (
            <Button variant="primary" onClick={onNext}>
              {t("activity.next")}
            </Button>
          ) : null
        ) : (
          <Button variant="primary" busy={busy} disabled={!canSubmit} onClick={onSubmit}>
            {busy ? t("activity.checking") : t("activity.check")}
          </Button>
        )}
      </div>
      <div className="activity__feedback" role="status" aria-live="polite">
        {result && state ? (
          <div className="activity__result" data-state={state} data-anim={state === "correct" ? "pop" : state === "wrong" ? "shake" : undefined}>
            <Mascot mood={mood} scale={3} />
            <div>
              <p className="activity__verdict">
                {state === "pending"
                  ? t("activity.pending")
                  : state === "correct"
                    ? t("activity.correct")
                    : state === "almost"
                      ? t("activity.almost")
                      : t("activity.wrong")}
              </p>
              {result.correctAnswer && state !== "correct" && state !== "pending" ? (
                <p>{t("activity.answer", { answer: result.correctAnswer })}</p>
              ) : null}
              {result.explanation ? <p>{t("activity.why", { text: result.explanation })}</p> : null}
              {result.sparks ? (
                <p className="activity__sparks" data-anim="slide-up">
                  {t("activity.sparks", { count: result.sparks })}
                </p>
              ) : null}
            </div>
          </div>
        ) : null}
      </div>
    </Panel>
  );
}
