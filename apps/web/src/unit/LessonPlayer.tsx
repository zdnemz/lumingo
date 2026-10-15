"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useApi } from "@/api/ApiProvider";
import { ActivityFrame } from "@/activities/ActivityFrame";
import { ErrorBanner } from "@/components/ErrorBanner";
import type { ActivityAnswer } from "@/generated/ActivityAnswer";
import type { ActivityResultView } from "@/generated/ActivityResultView";
import type { NextActivity } from "@/generated/NextActivity";
import type { SessionEnded } from "@/generated/SessionEnded";
import type { Skill } from "@/generated/Skill";
import type { Unit } from "@/generated/Unit";
import type { UnitSummaryView } from "@/generated/UnitSummaryView";
import { usePreferences, useT } from "@/state/PreferencesProvider";
import { useAction } from "@/state/useResource";
import { Button, LinkButton } from "@/ui/Button";
import { Panel } from "@/ui/Panel";
import { ActivityBodyView } from "./ActivityBodyView";
import { CheckpointPanel } from "./CheckpointPanel";
import { answerOf, emptyDraft, localized, type Draft } from "./model";
import { PresentationBody } from "./PresentationBody";
import { frameResult } from "./result";
import { RoleplayPanel } from "./RoleplayPanel";
import { UnavailableList } from "./UnavailableList";

/** The end of a unit run, as `POST /api/sessions/{id}/stop` reported it. */
function checkpointOf(ended: SessionEnded): UnitSummaryView | null {
  if (ended.feedback === null || ended.feedback.kind !== "unit") return null;
  return ended.feedback.summary;
}

/** The tone the frame draws an activity in, from the skill the unit gave it. */
function toneOf(skill: Skill): "listening" | "speaking" | "reading" | "writing" {
  return skill === "listening" || skill === "reading" || skill === "writing" ? skill : "speaking";
}

/**
 * The lesson walk. Every step comes from the server: the run is started with
 * `POST /api/sessions`, each activity is fetched with
 * `GET /api/sessions/{id}/next-activity`, an answer is handed in with
 * `POST /api/activities/submit`, and the checkpoint is settled with
 * `POST /api/sessions/{id}/stop`. The screen never decides what comes next, what
 * an activity is worth, or which activity cannot be done: it shows what the
 * server said, and the reasons it gave.
 *
 * When the server reports no activity left, the walk stops the session itself
 * and shows the checkpoint it settled on: the rows, the mean and the pass mark
 * exactly as they arrived.
 */
export function LessonPlayer({ unit, help, onAnswered }: { unit: Unit; help: boolean; onAnswered: (activityId: string) => void }) {
  const t = useT();
  const api = useApi();
  const { display } = usePreferences();
  const [sessionId, setSessionId] = useState<number | null>(null);
  const [step, setStep] = useState<NextActivity | null>(null);
  const [draft, setDraft] = useState<Draft>(emptyDraft);
  const [result, setResult] = useState<ActivityResultView | null>(null);
  const [ended, setEnded] = useState<SessionEnded | null>(null);
  const [roleplayStarted, setRoleplayStarted] = useState(false);
  const action = useAction();
  const run = action.run;
  const started = useRef(false);
  /** The step that failed last, so the banner's retry runs that step again and never a new session. */
  const retryStep = useRef<(() => void) | null>(null);

  /** Runs one server step, and remembers it as the step the banner retries when it fails. */
  const attempt = useCallback(
    (task: () => Promise<unknown>) => {
      retryStep.current = () => void run(task);
      return run(task);
    },
    [run],
  );

  /** Settles the run and shows the checkpoint the server gave. */
  const stop = useCallback(
    async (id: number) => {
      setEnded(await api.stopSession(id));
    },
    [api],
  );

  /** Asks for the next activity. When there is none, the server settles the run. */
  const advance = useCallback(
    async (id: number) => {
      const next = await api.nextActivity(id);
      setStep(next);
      setDraft(emptyDraft());
      setResult(null);
      setRoleplayStarted(false);
      if (next.activity === null) await attempt(() => stop(id));
    },
    [api, attempt, stop],
  );

  /** Starts the lesson session, then walks to the first activity. */
  const start = useCallback(async () => {
    const view = await api.startSession({
      kind: "lesson",
      unit_id: unit.id,
      activity_id: null,
      mode: null,
      topic: null,
      level: null,
      speak: null,
    });
    setSessionId(view.id);
    await attempt(() => advance(view.id));
  }, [api, attempt, advance, unit.id]);

  useEffect(() => {
    // React runs effects twice in development under Strict Mode. One run means one session.
    if (started.current) return;
    started.current = true;
    void attempt(start);
  }, [attempt, start]);

  const submit = useCallback(
    (answer: ActivityAnswer) => {
      const id = sessionId;
      const activity = step?.activity ?? null;
      if (id === null || activity === null) return;
      void attempt(async () => {
        const response = await api.submitActivity({ session_id: id, activity_id: activity.id, answer });
        if (response.result === null) {
          // `roleplay_start`: the conversation is running and has no result yet.
          setRoleplayStarted(true);
          return;
        }
        setResult(response.result);
        onAnswered(activity.id);
      });
    },
    [api, attempt, onAnswered, sessionId, step],
  );

  const onNext = useCallback(() => {
    if (sessionId !== null) void attempt(() => advance(sessionId));
  }, [advance, attempt, sessionId]);

  const activity = step?.activity ?? null;
  const body = activity?.body ?? null;
  const ready = activity === null ? null : answerOf(activity, draft);
  const checkpoint = useMemo(() => (ended === null ? null : checkpointOf(ended)), [ended]);
  const language = display.language;

  return (
    <div className="px-stack" style={{ "--gap": "var(--space-5)" } as React.CSSProperties}>
      {action.error ? (
        <>
          <ErrorBanner
            error={action.error}
            onRetry={() => {
              retryStep.current?.();
            }}
          />
          {/* The retry runs the step that failed again; this link is the way out when it keeps failing. */}
          <p>
            <LinkButton href="/unit/">{t("unit.again")}</LinkButton>
          </p>
        </>
      ) : null}
      {step === null && ended === null && action.error === null ? <p role="status">{t("unit.starting")}</p> : null}
      {ended !== null && checkpoint === null ? <p role="status">{t("unit.finishing")}</p> : null}

      {step !== null ? <p className="px-hint">{t("unit.run.progress", { answered: step.answered, total: step.total })}</p> : null}

      {/* The roleplay has its own panel: the conversation is started and finished by hand. */}
      {activity !== null && body !== null && body.kind === "roleplay" && result === null ? (
        <RoleplayPanel
          activity={activity}
          sessionId={sessionId}
          started={roleplayStarted}
          busy={action.busy}
          onStart={() => submit({ kind: "roleplay_start" })}
          onFinish={() => submit({ kind: "roleplay_finish" })}
        />
      ) : null}

      {/* The roleplay is over and the server scored it: the frame shows the verdict, and walks on. */}
      {activity !== null && body !== null && body.kind === "roleplay" && result !== null && result.outcome.kind !== "unscored" ? (
        <ActivityFrame
          skill="speaking"
          instructions={activity.instructions.en}
          result={frameResult(result, language)}
          canSubmit={false}
          onSubmit={() => undefined}
          onNext={onNext}
        >
          <p className="unit-sentence px-panel px-panel--inset">{localized(body.scenario, language)}</p>
        </ActivityFrame>
      ) : null}

      {/* An activity this screen has no control for: the task as text, and the screen says so. */}
      {activity !== null && ready?.noControl === true && body !== null && body.kind !== "roleplay" ? (
        <Panel title={activity.instructions.en} tone={toneOf(activity.skill)} raised>
          <PresentationBody activity={activity} />
        </Panel>
      ) : null}

      {activity !== null && ready?.noControl === false && result?.outcome.kind !== "unscored" && body !== null && body.kind !== "roleplay" ? (
        <ActivityFrame
          skill={toneOf(activity.skill)}
          instructions={activity.instructions.en}
          help={help && language === "en" ? activity.instructions.id : undefined}
          result={result === null ? undefined : frameResult(result, language)}
          canSubmit={ready.canSubmit}
          busy={action.busy}
          onSubmit={() => {
            if (ready.answer !== null) submit(ready.answer);
          }}
          onNext={result === null ? undefined : onNext}
        >
          <ActivityBodyView
            activity={activity}
            draft={draft}
            locked={result !== null}
            onChange={(patch) => setDraft((current) => ({ ...current, ...patch }))}
          />
        </ActivityFrame>
      ) : null}

      {/* A result the frame cannot show: the server stored the answer and said why it has no score. */}
      {activity !== null && result !== null && result.outcome.kind === "unscored" ? (
        <Panel title={activity.instructions.en} tone="primary" raised>
          <div className="px-stack" style={{ "--gap": "var(--space-3)" } as React.CSSProperties}>
            <p>{result.outcome.reason.trim() !== "" ? result.outcome.reason : t("activity.unscored")}</p>
            <p>
              <Button variant="primary" onClick={onNext}>
                {t("activity.next")}
              </Button>
            </p>
          </div>
        </Panel>
      ) : null}

      {checkpoint !== null ? <CheckpointPanel result={checkpoint} unit={unit} /> : null}
      {ended !== null && checkpoint === null ? (
        <Panel tone="warning" role="note">
          <p>{t("unit.checkpoint.none")}</p>
        </Panel>
      ) : null}

      {ended !== null ? (
        <p>
          <LinkButton href="/unit/">{t("unit.again")}</LinkButton>
        </p>
      ) : null}

      <UnavailableList unavailable={step?.unavailable ?? []} unit={unit} />
    </div>
  );
}
