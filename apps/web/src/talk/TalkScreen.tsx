"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { useApi } from "@/api/ApiProvider";
import { useServerState } from "@/api/ServerState";
import { ErrorBanner } from "@/components/ErrorBanner";
import { UnavailableNote } from "@/components/UnavailableNote";
import type { ConversationSummaryView } from "@/generated/ConversationSummaryView";
import type { FeedbackMode } from "@/generated/FeedbackMode";
import type { Level } from "@/generated/Level";
import type { MessageKey } from "@/i18n";
import { useT } from "@/state/PreferencesProvider";
import { useAction } from "@/state/useResource";
import { Button } from "@/ui/Button";
import { Panel } from "@/ui/Panel";
import { Segmented } from "@/ui/Segmented";
import { TextField } from "@/ui/TextField";
import { ChatInput } from "./ChatInput";
import { ChatLog } from "./ChatLog";
import { ConversationSummary } from "./Summary";
import { EMPTY, LEVELS, MODES, applyEvent, seed, sessionIdOf, summaryOf, type Transcript } from "./model";

/** Typing is what this screen does; the voice half is reported as missing instead of started. */
type Channel = "text" | "voice";

const CHANNEL_TEXT: Record<Channel, MessageKey> = {
  text: "talk.channel.text",
  voice: "talk.channel.voice",
};

const MODE_TEXT: Record<FeedbackMode, MessageKey> = {
  fluency: "talk.mode.fluency",
  accuracy: "talk.mode.accuracy",
};

const CHANNELS: readonly Channel[] = ["text", "voice"];

/**
 * The conversation screen.
 *
 * The screen holds no conversation of its own: `POST /api/sessions` starts a
 * text chat, the server's own events carry every line (`TranscriptFinal` for
 * the learner, `TutorTextDelta` for the tutor) and a reloaded page rebuilds
 * what it missed from the snapshot's `recent` lines. Offline the start is
 * refused with a coded answer, which is shown as it is: the refusal is the
 * honest state of a program without a provider, and the setup guide the
 * connection banner offers is the way out.
 */
export function TalkScreen() {
  const t = useT();
  const api = useApi();
  const { snapshot, events } = useServerState();
  const action = useAction();
  const [channel, setChannel] = useState<Channel>("text");
  const [mode, setMode] = useState<FeedbackMode>("fluency");
  const [topic, setTopic] = useState("");
  const [level, setLevel] = useState<Level>("A1");
  const [sessionId, setSessionId] = useState<number | null>(null);
  const [transcript, setTranscript] = useState<Transcript>(EMPTY);
  const [summary, setSummary] = useState<ConversationSummaryView | null>(null);
  const [ended, setEnded] = useState(false);
  /** The server's own sentence when a session cannot go on. */
  const [note, setNote] = useState<string | null>(null);
  const [working, setWorking] = useState<"start" | "send" | "finish" | null>(null);

  /** The session on screen. A ref, because events arrive outside a render. */
  const sessionRef = useRef<number | null>(null);
  /** The newest event folded in, so nothing is applied twice. */
  const lastSeq = useRef(0);
  /** The session whose snapshot lines were used, so a later snapshot does not replace newer lines. */
  const seededFor = useRef<number | null>(null);

  /** Follows a session without touching the lines: the snapshot already carries them. */
  const follow = useCallback((id: number) => {
    sessionRef.current = id;
    setSessionId(id);
  }, []);

  /** A conversation begins. The lines of any earlier one are over, and so is its summary. */
  const begin = useCallback(
    (id: number) => {
      follow(id);
      seededFor.current = id;
      setTranscript(EMPTY);
      setSummary(null);
      setEnded(false);
      setNote(null);
    },
    [follow],
  );

  // The reconnect contract: a page that was reloaded mid-conversation rebuilds
  // it from the snapshot, because the events that made those lines are gone.
  useEffect(() => {
    const active = snapshot?.active_session ?? null;
    if (active === null || seededFor.current === active.id) return;
    seededFor.current = active.id;
    follow(active.id);
    setTranscript(seed(active.recent));
  }, [snapshot, follow]);

  // The events, folded in the order they arrived. The stream keeps only a few
  // of them, so the list is grown as they come rather than derived from the
  // window; older lines come from the snapshot and nowhere else.
  useEffect(() => {
    const fresh = events.filter((event) => event.seq > lastSeq.current).reverse();
    if (fresh.length === 0) return;
    lastSeq.current = Math.max(...fresh.map((event) => event.seq));

    // The answer to `startSession` may not have been read yet: the server's own
    // event names the session it opened, so the opening turn is never lost.
    for (const event of fresh) {
      if (event.type === "SessionState" && event.life === "active" && event.session_id !== sessionRef.current) {
        begin(event.session_id);
        break;
      }
    }

    const mine = fresh.filter((event) => sessionIdOf(event) === sessionRef.current);
    if (mine.length === 0) return;
    setTranscript((current) => mine.reduce(applyEvent, current));
    const conversation = mine.reduce<ConversationSummaryView | null>(
      (found, event) => summaryOf(event) ?? found,
      null,
    );
    if (conversation !== null) setSummary(conversation);
    const sentence = mine.reduce<string | null>(
      (found, event) => (event.type === "SessionState" && event.message !== null ? event.message : found),
      null,
    );
    if (sentence !== null) setNote(sentence);
    if (mine.some((event) => event.type === "SessionState" && event.life === "ended")) setEnded(true);
  }, [events, begin]);

  const run = useCallback(
    async (what: "start" | "send" | "finish", task: () => Promise<unknown>) => {
      setWorking(what);
      try {
        return await action.run(task);
      } finally {
        setWorking(null);
      }
    },
    [action],
  );

  /** Starts a text chat. A refusal (offline: `provider_not_configured`) is shown, not hidden. */
  const start = useCallback(() => {
    const text = topic.trim();
    if (text === "" || working !== null) return;
    void run("start", async () => {
      const view = await api.startSession({
        kind: "text_chat",
        unit_id: null,
        activity_id: null,
        mode,
        topic: { kind: "typed", text },
        level,
        speak: null,
      });
      // The opening events can arrive before this answer is read; then the
      // session is followed already and its lines must not be cleared again.
      if (sessionRef.current !== view.id) begin(view.id);
    });
  }, [api, begin, level, mode, run, topic, working]);

  const send = useCallback(
    (text: string) => {
      const id = sessionRef.current;
      if (id === null || working !== null) return;
      void run("send", () => api.sendText(id, text));
    },
    [api, run, working],
  );

  const finish = useCallback(() => {
    const id = sessionRef.current;
    if (id === null || working !== null) return;
    void run("finish", async () => {
      const stopped = await api.stopSession(id);
      setEnded(true);
      if (stopped.feedback !== null && stopped.feedback.kind === "conversation") setSummary(stopped.feedback.summary);
    });
  }, [api, run, working]);

  const running = sessionId !== null && !ended;
  const ready = topic.trim() !== "" && working === null;
  const channels = CHANNELS.map((value) => ({ value, label: t(CHANNEL_TEXT[value]) }));
  const modes = MODES.map((value) => ({ value, label: t(MODE_TEXT[value]) }));
  const levels = LEVELS.map((value) => ({ value, label: value }));

  return (
    <div className="px-stack" style={{ "--gap": "var(--space-5)" } as React.CSSProperties}>
      <p className="px-hint">{t("talk.intro")}</p>

      {action.error ? <ErrorBanner error={action.error} /> : null}

      {!running ? (
        <div className="px-stack" style={{ "--gap": "var(--space-4)" } as React.CSSProperties}>
          <Segmented legend={t("talk.channel")} value={channel} options={channels} onChange={setChannel} />

          {channel === "voice" ? (
            // No control starts a voice chat: the server would refuse the start
            // with `provider_not_configured`, not with the missing speech, so a
            // button here could not show an honest answer.
            <UnavailableNote feature="speech" />
          ) : (
            <>
              <Segmented legend={t("talk.mode")} value={mode} options={modes} onChange={setMode} hint={t("talk.mode.hint")} />
              <TextField
                label={t("talk.topic")}
                value={topic}
                onChange={setTopic}
                placeholder={t("talk.topic.placeholder")}
                hint={t("talk.topic.hint")}
              />
              <Segmented legend={t("talk.level")} value={level} options={levels} onChange={setLevel} hint={t("talk.level.hint")} />
              <div className="px-row">
                <Button variant="primary" onClick={start} busy={working === "start"} disabled={!ready}>
                  {working === "start" ? t("talk.starting") : t("talk.start")}
                </Button>
              </div>
            </>
          )}
        </div>
      ) : null}

      {note !== null ? (
        <Panel tone="warning" role="note">
          <p>{note}</p>
        </Panel>
      ) : null}

      {sessionId !== null ? (
        <Panel title={t("talk.conversation")} raised>
          <div className="px-stack" style={{ "--gap": "var(--space-4)" } as React.CSSProperties}>
            {transcript.messages.length === 0 ? <p className="px-hint">{t("talk.waiting")}</p> : null}
            <ChatLog messages={transcript.messages} thinking={transcript.thinking} />
            {running ? <ChatInput onSend={send} busy={working === "send"} /> : null}
          </div>
        </Panel>
      ) : null}

      {running ? (
        <div className="px-row">
          <Button onClick={finish} busy={working === "finish"}>
            {working === "finish" ? t("talk.finishing") : t("talk.finish")}
          </Button>
        </div>
      ) : null}

      {ended && summary === null ? <p role="status">{t("talk.ended")}</p> : null}
      {summary !== null ? <ConversationSummary summary={summary} /> : null}
    </div>
  );
}
