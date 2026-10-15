"use client";

import { useCallback, useState } from "react";
import { McqActivity } from "@/activities/McqActivity";
import { useApi } from "@/api/ApiProvider";
import { ErrorBanner } from "@/components/ErrorBanner";
import { UnavailableNote } from "@/components/UnavailableNote";
import type { Level } from "@/generated/Level";
import type { ReadingFallbackReasonView } from "@/generated/ReadingFallbackReasonView";
import type { ReadingOutcomeView } from "@/generated/ReadingOutcomeView";
import type { ReadingScoreView } from "@/generated/ReadingScoreView";
import type { MessageKey } from "@/i18n";
import { usePreferences, useT } from "@/state/PreferencesProvider";
import { useAction } from "@/state/useResource";
import { Badge } from "@/ui/Badge";
import { Button } from "@/ui/Button";
import { Panel } from "@/ui/Panel";
import { Segmented } from "@/ui/Segmented";
import { StatusBanner } from "@/ui/StatusBanner";
import { TextField } from "@/ui/TextField";
import { GlossedText } from "./GlossedText";
import { answersOf, DEFAULT_LEVEL, explanationText, LEVELS, offeredReadings, targetOf, type OfferedReading } from "./model";

/** Why the provider's text was not shown, as the server named it. */
const REASON: Record<ReadingFallbackReasonView, MessageKey> = {
  provider_unavailable: "read.reason.provider_unavailable",
  unusable: "read.reason.unusable",
};

const LEVEL_OPTIONS = LEVELS.map((level) => ({ value: level, label: level }));

/**
 * Graded reading. The learner picks a level and a topic, and the server answers
 * with either a text a model wrote or, when no text could be written, the
 * reading sets already authored in the units of that level.
 *
 * The screen invents nothing: it shows the texts the server sent, scores the
 * answers with the route the outcome names, and repeats the server's own
 * sentences when a text could not be written. The level is fixed when the
 * session starts, because the server reads it then, so it is shown as a fact
 * afterwards instead of a picker that would do nothing. The answers of the
 * running session are read from the HTTP answer and never from the
 * `FeedbackReady` event, which carries the same score.
 */
export function ReadScreen() {
  const t = useT();
  const api = useApi();
  const [level, setLevel] = useState<Level>(DEFAULT_LEVEL);
  const [topic, setTopic] = useState("");
  /** The running reading session, if one was started. */
  const [sessionId, setSessionId] = useState<number | null>(null);
  /** The level the running session reads at, which the server fixed when it started. */
  const [sessionLevel, setSessionLevel] = useState<Level | null>(null);
  const [outcome, setOutcome] = useState<ReadingOutcomeView | null>(null);
  /** Which text this is, so a new one starts unanswered even when it is the same set. */
  const [round, setRound] = useState(0);
  const action = useAction();

  /**
   * Starts the session the first time and asks for a text every time. The
   * session is reused for "another text": one screen means one session, and a
   * second session while the first runs would be refused.
   */
  const getText = useCallback(() => {
    void action.run(async () => {
      let id = sessionId;
      if (id === null) {
        const view = await api.startSession({
          kind: "reading",
          unit_id: null,
          activity_id: null,
          mode: null,
          topic: null,
          level,
          speak: null,
        });
        id = view.id;
        setSessionId(id);
        setSessionLevel(level);
      }
      setOutcome(await api.generateReading({ session_id: id, topic: { kind: "typed", text: topic.trim() } }));
      setRound((value) => value + 1);
    });
  }, [action, api, level, sessionId, topic]);

  const blank = topic.trim() === "";

  return (
    <div className="px-stack" style={{ "--gap": "var(--space-5)" } as React.CSSProperties}>
      <Panel title={t("read.setup.title")} raised className="read-setup">
        <p>{t("read.intro")}</p>
        {sessionLevel === null ? (
          <Segmented legend={t("read.level")} value={level} options={LEVEL_OPTIONS} onChange={setLevel} />
        ) : (
          <p className="px-hint">{t("read.level.fixed", { level: sessionLevel })}</p>
        )}
        <TextField
          label={t("read.topic")}
          value={topic}
          onChange={setTopic}
          placeholder={t("read.topic.placeholder")}
          hint={t("read.topic.hint")}
        />
        <div className="px-row">
          <Button variant="primary" busy={action.busy} disabled={blank} onClick={getText}>
            {action.busy ? t("read.starting") : sessionId === null ? t("read.start") : t("read.again")}
          </Button>
        </div>
      </Panel>

      {action.error ? <ErrorBanner error={action.error} onRetry={getText} /> : null}

      {/* Reading a text aloud needs the speech engine, which this build does not have. */}
      <UnavailableNote feature="speech" />

      {outcome === null ? null : <Offered key={round} outcome={outcome} sessionId={sessionId} />}
    </div>
  );
}

/** Everything the server offered for the topic: a generated text, or the authored sets. */
function Offered({ outcome, sessionId }: { outcome: ReadingOutcomeView; sessionId: number | null }) {
  const t = useT();
  const readings = offeredReadings(outcome);
  if (sessionId === null) return null;

  if (readings.length === 0) {
    return (
      <StatusBanner tone="info" title={t("read.empty.title")}>
        <p>{t("read.empty.body")}</p>
      </StatusBanner>
    );
  }

  if (outcome.kind === "generated") {
    return (
      <div className="px-stack" style={{ "--gap": "var(--space-5)" } as React.CSSProperties}>
        {readings.map((reading) => (
          <Reading key={reading.key} reading={reading} sessionId={sessionId} />
        ))}
      </div>
    );
  }

  return (
    <Panel title={t("read.fallback.title")} raised>
      <p className="px-hint">{t(REASON[outcome.reason])}</p>
      <div className="px-stack" style={{ "--gap": "var(--space-5)" } as React.CSSProperties}>
        {readings.map((reading) => (
          <Reading key={reading.key} reading={reading} sessionId={sessionId} />
        ))}
      </div>
    </Panel>
  );
}

/** One text and its questions, answered and scored on their own. */
function Reading({ reading, sessionId }: { reading: OfferedReading; sessionId: number }) {
  const t = useT();
  const api = useApi();
  const { display } = usePreferences();
  const questions = reading.text.questions;
  const [picks, setPicks] = useState<(number | null)[]>(() => questions.map(() => null));
  const [openWord, setOpenWord] = useState<string | null>(null);
  const [score, setScore] = useState<ReadingScoreView | null>(null);
  const action = useAction();

  /** Hands in exactly one answer per question, `null` for one left open. */
  const check = useCallback(() => {
    void action.run(async () => {
      const answered = await api.answerReading(sessionId, {
        target: targetOf(reading),
        answers: answersOf(questions.length, picks),
      });
      setScore(answered.score);
    });
  }, [action, api, picks, questions.length, reading, sessionId]);

  // An authored set carries no glossary, so only a generated text has words to open.
  const glossary = reading.text.glossary.map((entry) => ({ word: entry.word, gloss: entry.gloss_l1 }));

  return (
    <Panel as="article" tone="reading" className="read-text">
      <h3 className="read-text__title">{reading.text.title}</h3>
      {reading.kind === "generated" ? (
        <div className="read-text__labels">
          <Badge tone="info">{t("read.generated.label")}</Badge>
          <p className="px-hint">{t("read.generated.note")}</p>
          {reading.regenerated ? <p className="px-hint">{t("read.generated.regenerated")}</p> : null}
          {reading.vocabularyChecked ? null : <p className="px-hint">{t("read.generated.vocabulary.skipped")}</p>}
        </div>
      ) : (
        <p className="px-hint">{t("read.set.from", { unit: reading.unitId })}</p>
      )}

      <GlossedText text={reading.text.passage} glossary={glossary} openWord={openWord} onOpen={setOpenWord} />

      <ol className="read-questions">
        {questions.map((question, index) => {
          const result = score?.questions[index] ?? null;
          return (
            <li key={index} className="read-question">
              <McqActivity
                stem={question.stem}
                options={question.options}
                selected={picks[index] ?? null}
                locked={score !== null}
                correctIndex={result?.correct_index}
                onSelect={(choice) => setPicks((current) => current.map((value, at) => (at === index ? choice : value)))}
              />
              {result ? <p className="read-question__why">{t("activity.why", { text: explanationText(result, display.language) })}</p> : null}
            </li>
          );
        })}
      </ol>

      {score === null ? (
        <div className="px-row">
          <Button variant="primary" busy={action.busy} onClick={check}>
            {action.busy ? t("activity.checking") : t("activity.check")}
          </Button>
        </div>
      ) : (
        <div className="read-score" role="status">
          <p className="read-score__line">{t("read.score", { correct: score.correct, total: score.total })}</p>
          <p className="px-hint">{t("read.score.note")}</p>
        </div>
      )}

      {action.error ? <ErrorBanner error={action.error} onRetry={check} /> : null}
    </Panel>
  );
}
