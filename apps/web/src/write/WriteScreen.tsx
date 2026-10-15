"use client";

import { useCallback, useMemo, useRef, useState } from "react";
import { useApi } from "@/api/ApiProvider";
import { useServerState } from "@/api/ServerState";
import { ErrorBanner } from "@/components/ErrorBanner";
import type { ActiveSessionView } from "@/generated/ActiveSessionView";
import type { DraftAccepted } from "@/generated/DraftAccepted";
import type { DraftFeedbackView } from "@/generated/DraftFeedbackView";
import type { SessionEnded } from "@/generated/SessionEnded";
import { useT } from "@/state/PreferencesProvider";
import { useAction } from "@/state/useResource";
import { Button } from "@/ui/Button";
import { Panel } from "@/ui/Panel";
import { Segmented } from "@/ui/Segmented";
import { DraftPanel } from "./DraftPanel";
import { LEVELS, draftFeedbackByTurn, type WriteLevel } from "./model";

/** One draft of this session: the text that was handed in, and the server's answer to it. */
interface Draft {
  key: number;
  text: string;
  accepted: DraftAccepted;
}

/** No feedback yet: a shared empty map, so it does not look like a change on every render. */
const NOTHING_SEEN: ReadonlyMap<number, DraftFeedbackView> = new Map();

/**
 * The writing workshop.
 *
 * A session is started with the learner's own prompt, because a bank prompt
 * needs `catalogs/topics.json`, which this build does not ship. The server
 * refuses a session with no prompt and a draft with no text, so the editor
 * sends neither.
 *
 * Layer one — the word count and the rule checker's findings — comes back from
 * `POST /api/writing/{session}/drafts` at once and needs no provider. Layers
 * two and three arrive detached as a `FeedbackReady` event, so they are keyed
 * by `turn_seq` and joined with the draft they belong to when the screen draws:
 * an event that arrives before the HTTP answer is not lost.
 */
export function WriteScreen() {
  const t = useT();
  const api = useApi();
  const { events } = useServerState();
  const action = useAction();
  const run = action.run;

  const [prompt, setPrompt] = useState("");
  const [level, setLevel] = useState<WriteLevel>("A1");
  const [session, setSession] = useState<ActiveSessionView | null>(null);
  const [started, setStarted] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [drafts, setDrafts] = useState<readonly Draft[]>([]);
  const [ended, setEnded] = useState<SessionEnded | null>(null);
  /**
   * Every `FeedbackReady` of this session, by `turn_seq`. The stream holds only
   * its newest events, so what has been seen is kept: a draft's feedback does
   * not disappear from the screen when the stream moves on.
   */
  const [seen, setSeen] = useState<ReadonlyMap<number, DraftFeedbackView>>(NOTHING_SEEN);
  const [lastStream, setLastStream] = useState<ReadonlyMap<number, DraftFeedbackView>>(NOTHING_SEEN);
  const nextKey = useRef(0);
  /** The step that failed last, so the banner's retry runs that step again and never a different one. */
  const retryStep = useRef<(() => void) | null>(null);

  const sessionId = session?.id ?? null;
  const active = session !== null && ended === null;
  const fromStream = useMemo(
    () => (sessionId === null ? NOTHING_SEEN : draftFeedbackByTurn(events, sessionId)),
    [events, sessionId],
  );
  if (lastStream !== fromStream) {
    // The newest feedback for a draft replaces what was kept for it.
    setLastStream(fromStream);
    setSeen((current) => {
      let changed = false;
      const next = new Map(current);
      for (const [turn, view] of fromStream) {
        if (next.get(turn) !== view) {
          next.set(turn, view);
          changed = true;
        }
      }
      return changed ? next : current;
    });
  }

  /** Runs one server step, and remembers it as the step the banner retries when it fails. */
  const attempt = useCallback(
    (task: () => Promise<unknown>) => {
      retryStep.current = () => void run(task);
      return run(task);
    },
    [run],
  );

  const start = useCallback(() => {
    const text = prompt.trim();
    // A session with no topic is refused, so the screen never asks for one.
    if (text === "") return;
    void attempt(async () => {
      const view = await api.startSession({
        kind: "writing",
        unit_id: null,
        activity_id: null,
        mode: null,
        topic: { kind: "typed", text },
        level,
        speak: null,
      });
      setSession(view);
      setStarted(text);
      setDrafts([]);
      setSeen(NOTHING_SEEN);
      setLastStream(NOTHING_SEEN);
      setEnded(null);
      setDraft("");
    });
  }, [api, attempt, level, prompt]);

  const check = useCallback(() => {
    const id = sessionId;
    const text = draft.trim();
    // An empty draft is refused, so the text stays in the editor until the server takes it.
    if (id === null || text === "") return;
    void attempt(async () => {
      const accepted = await api.submitDraft(id, text);
      nextKey.current += 1;
      setDrafts((current) => [...current, { key: nextKey.current, text, accepted }]);
      setDraft("");
    });
  }, [api, attempt, draft, sessionId]);

  const finish = useCallback(() => {
    const id = sessionId;
    if (id === null) return;
    void attempt(async () => {
      setEnded(await api.stopSession(id));
    });
  }, [api, attempt, sessionId]);

  const again = useCallback(() => {
    setSession(null);
    setStarted(null);
    setEnded(null);
    setDrafts([]);
    setSeen(NOTHING_SEEN);
    setLastStream(NOTHING_SEEN);
    setDraft("");
    setPrompt("");
  }, []);

  const levels = useMemo(() => LEVELS.map((value) => ({ value, label: value })), []);

  return (
    <div className="px-stack write" style={{ "--gap": "var(--space-5)" } as React.CSSProperties}>
      {action.error ? (
        <ErrorBanner
          error={action.error}
          onRetry={() => {
            retryStep.current?.();
          }}
        />
      ) : null}

      {session === null ? (
        <Panel tone="writing" raised className="px-stack">
          <p className="write__task">{t("write.intro")}</p>
          <div>
            <label className="px-label" htmlFor="write-prompt">
              {t("write.prompt.label")}
            </label>
            <textarea
              id="write-prompt"
              className="px-field"
              rows={3}
              value={prompt}
              aria-describedby="write-prompt-hint"
              onChange={(event) => setPrompt(event.target.value)}
            />
            <p className="px-hint" id="write-prompt-hint">
              {t("write.prompt.hint")}
            </p>
          </div>
          <Segmented legend={t("write.level.legend")} value={level} options={levels} onChange={setLevel} />
          <p>
            <Button variant="primary" busy={action.busy} disabled={prompt.trim() === ""} onClick={start}>
              {t("write.start")}
            </Button>
          </p>
        </Panel>
      ) : null}

      {started !== null && active ? (
        <Panel tone="writing" title={t("write.prompt.label")}>
          <p className="write__task">{started}</p>
        </Panel>
      ) : null}

      {active ? (
        <Panel tone="writing" raised className="px-stack">
          <div>
            <label className="px-label" htmlFor="write-draft">
              {t("write.draft.label")}
            </label>
            <textarea
              id="write-draft"
              className="px-field"
              rows={6}
              value={draft}
              onChange={(event) => setDraft(event.target.value)}
            />
          </div>
          <p className="px-row">
            <Button variant="primary" busy={action.busy} disabled={draft.trim() === ""} onClick={check}>
              {t("write.check")}
            </Button>
            <Button onClick={finish}>{t("write.finish")}</Button>
          </p>
        </Panel>
      ) : null}

      {drafts.map((entry) => (
        <DraftPanel
          key={entry.key}
          accepted={entry.accepted}
          text={entry.text}
          feedback={seen.get(entry.accepted.turn_seq) ?? null}
        />
      ))}

      {ended !== null ? (
        <Panel tone="success" raised className="px-stack" role="note">
          <h2>{t("write.ended.title")}</h2>
          <p>{t("write.ended.none")}</p>
          <p>
            <Button variant="primary" onClick={again}>
              {t("write.again")}
            </Button>
          </p>
        </Panel>
      ) : null}
    </div>
  );
}
