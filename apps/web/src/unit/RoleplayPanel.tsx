"use client";

import { useMemo } from "react";
import { useServerState } from "@/api/ServerState";
import type { ActivityPresentation } from "@/generated/ActivityPresentation";
import { usePreferences, useT } from "@/state/PreferencesProvider";
import { Badge } from "@/ui/Badge";
import { Button } from "@/ui/Button";
import { Panel } from "@/ui/Panel";
import { localized } from "./model";

/** One line of the conversation, from the events the server sent for this session. */
interface Line {
  key: string;
  who: "learner" | "tutor";
  text: string;
}

/**
 * The conversation as the event stream told it. The lines come from the
 * server's own events (`TranscriptFinal` for the learner, `TutorSentenceSpoken`
 * and `TutorTextDelta` for the tutor), so nothing is invented here: a turn that
 * never happened has no line.
 */
function useConversation(sessionId: number | null): Line[] {
  const { events } = useServerState();
  return useMemo(() => {
    if (sessionId === null) return [];
    const lines: Line[] = [];
    const deltas = new Map<number, string>();
    // `events` is newest first.
    for (const event of [...events].reverse()) {
      if (!("session_id" in event) || event.session_id !== sessionId) continue;
      if (event.type === "TranscriptFinal") {
        lines.push({ key: `l-${event.seq}`, who: "learner", text: event.text });
      }
      if (event.type === "TutorSentenceSpoken") {
        lines.push({ key: `t-${event.seq}`, who: "tutor", text: event.text });
      }
      if (event.type === "TutorTextDelta") {
        const text = (deltas.get(event.turn) ?? "") + event.delta;
        deltas.set(event.turn, text);
      }
    }
    for (const [turn, text] of deltas) {
      if (text.trim() !== "" && !lines.some((line) => line.who === "tutor" && line.text.startsWith(text.slice(0, 24)))) {
        lines.push({ key: `d-${turn}`, who: "tutor", text });
      }
    }
    return lines;
  }, [events, sessionId]);
}

/** The sentence the server sent when the session could not reach the provider. */
function useProviderMessage(sessionId: number | null): string | null {
  const { events } = useServerState();
  return useMemo(() => {
    for (const event of events) {
      if (event.type !== "SessionState" || event.session_id !== sessionId) continue;
      if (event.life === "provider_unavailable" && event.message !== null) return event.message;
    }
    return null;
  }, [events, sessionId]);
}

export interface RoleplayPanelProps {
  activity: ActivityPresentation;
  sessionId: number | null;
  /** The roleplay was started and has not been finished. */
  started: boolean;
  busy: boolean;
  onStart: () => void;
  onFinish: () => void;
}

/**
 * The roleplay of the unit. The walk starts the conversation with
 * `roleplay_start`, which hands the turn to the engine: the tutor greets the
 * learner first and then the learner answers. Offline the engine reports that
 * no provider answered, and the panel shows the server's own sentence and the
 * "Finish the conversation" control, which submits `roleplay_finish` so the
 * walk can carry on.
 */
export function RoleplayPanel({ activity, sessionId, started, busy, onStart, onFinish }: RoleplayPanelProps) {
  const t = useT();
  const { display } = usePreferences();
  const lines = useConversation(sessionId);
  const providerMessage = useProviderMessage(sessionId);
  const body = activity.body;
  if (body.kind !== "roleplay") return null;

  return (
    <Panel title={localized(body.scenario, display.language)} raised>
      <div className="px-stack" style={{ "--gap": "var(--space-3)" } as React.CSSProperties}>
        <p>
          {t("unit.roleplay.roles", { learner: body.learner_role, tutor: body.tutor_role })}
        </p>
        <p className="px-hint">{t("unit.roleplay.turns", { count: body.max_turns })}</p>

        <section className="px-stack" style={{ "--gap": "var(--space-2)" } as React.CSSProperties}>
          <h3>{t("unit.roleplay.goals")}</h3>
          <ul className="unit-objectives">
            {body.goals.map((goal, index) => (
              <li key={index}>{goal}</li>
            ))}
          </ul>
        </section>

        {providerMessage !== null ? (
          <div className="unavailable" role="alert">
            <p className="unavailable__title">
              <Badge tone="info">{t("unit.unavailable.title")}</Badge>
            </p>
            <p className="px-hint">{providerMessage}</p>
          </div>
        ) : null}

        {lines.length > 0 ? (
          <ol className="unit-dialogue__turns">
            {lines.map((line) => (
              <li key={line.key}>
                <span className="unit-dialogue__speaker">{line.who === "learner" ? t("chat.you") : t("chat.tutor")}</span>
                <span>{line.text}</span>
              </li>
            ))}
          </ol>
        ) : null}

        {started ? (
          <Button onClick={onFinish} busy={busy}>
            {t("unit.roleplay.finish")}
          </Button>
        ) : (
          <Button variant="primary" onClick={onStart} busy={busy}>
            {t("unit.roleplay.start")}
          </Button>
        )}
      </div>
    </Panel>
  );
}
