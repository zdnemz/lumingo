"use client";

import { useCallback, useEffect, useRef, useState, type FormEvent } from "react";
import { useApi } from "@/api/ApiProvider";
import type { AttemptList } from "@/generated/AttemptList";
import type { SkillEstimateView } from "@/generated/SkillEstimateView";
import { useFormatTime } from "@/providers/ProviderCard";
import { useT } from "@/state/PreferencesProvider";
import { useResource } from "@/state/useResource";
import { Button } from "@/ui/Button";
import { Panel } from "@/ui/Panel";
import { TextField } from "@/ui/TextField";
import { AttemptEvidencePanel } from "./AttemptEvidencePanel";
import { confidenceBand, profileRow, type ProfileSkill } from "./model";

/** A positive whole number, written with digits only. Anything else is not an attempt number. */
export function parseAttemptNumber(text: string): number | null {
  const trimmed = text.trim();
  if (!/^\d{1,15}$/.test(trimmed)) return null;
  const value = Number(trimmed);
  return Number.isSafeInteger(value) && value > 0 ? value : null;
}

/**
 * The counted attempts behind one skill, from
 * `GET /api/attempts?skill=...`, newest first. Only counted work is listed, so
 * a learner whose work was all free practice sees the honest empty state, not
 * an error. Each row is a control that opens the stored evidence of that
 * attempt, so the quotes and versions are reachable without typing a number.
 * A row waiting for the provider is shown as waiting, never as a score.
 */
function SkillAttempts({ skill, onOpen }: { skill: ProfileSkill; onOpen: (id: number) => void }) {
  const t = useT();
  const formatTime = useFormatTime();
  const api = useApi();
  const load = useCallback((signal: AbortSignal) => api.listAttempts(skill, signal), [api, skill]);
  const attempts = useResource<AttemptList>(load);
  const state = attempts.state;

  return (
    <>
      <p className="px-hint">{t("progress.drill.attempts.intro")}</p>
      {state.status === "loading" ? <p role="status">{t("progress.drill.attempts.loading")}</p> : null}
      {state.status === "error" ? (
        <div className="px-stack" style={{ "--gap": "var(--space-2)" } as React.CSSProperties} role="alert">
          <p>{t("progress.drill.attempts.error")}</p>
          <div className="px-row">
            <Button onClick={attempts.reload}>{t("state.retry")}</Button>
          </div>
        </div>
      ) : null}
      {state.status === "ready" ? (
        state.data.attempts.length === 0 ? (
          <p>{t("progress.drill.attempts.empty")}</p>
        ) : (
          <>
            <ul className="rows attempts">
              {state.data.attempts.map((row) => (
                <li key={row.id} className="rows__item attempts__item">
                  <Button
                    className="attempts__open"
                    aria-label={t("progress.drill.attempts.open", { id: row.id })}
                    onClick={() => onOpen(row.id)}
                  >
                    <span className="rows__text">
                      {t("progress.drill.attempts.row", {
                        id: row.id,
                        activity: row.activity_id,
                        status: row.status,
                        time: formatTime(row.created_at),
                      })}
                    </span>
                    <span className="px-hint attempts__facts">
                      <span className="phonetic">{row.activity_type}</span>
                      <span className="phonetic">{row.dimension}</span>
                      {row.status === "pending_llm" ? (
                        <span>{t("progress.drill.attempts.pending")}</span>
                      ) : row.normalized !== null ? (
                        <span className="phonetic">{row.normalized}</span>
                      ) : null}
                    </span>
                  </Button>
                </li>
              ))}
            </ul>
            {state.data.truncated ? <p className="px-hint">{t("progress.drill.attempts.truncated")}</p> : null}
          </>
        )
      ) : null}
    </>
  );
}

export interface SkillDrillDownProps {
  skill: ProfileSkill;
  estimate: SkillEstimateView | undefined;
  onClose: () => void;
}

/**
 * From one skill's estimate to the work behind it. The facts come from the
 * estimate row (status, count, rule version), and the counted attempts come
 * from `GET /api/attempts?skill=...`; one of them opens the stored evidence.
 * The lookup field still reaches any attempt number the list does not hold.
 */
export function SkillDrillDown({ skill, estimate, onClose }: SkillDrillDownProps) {
  const t = useT();
  const formatTime = useFormatTime();
  const heading = useRef<HTMLHeadingElement>(null);
  const [text, setText] = useState("");
  const [invalid, setInvalid] = useState(false);
  const [attempt, setAttempt] = useState<number | null>(null);

  useEffect(() => {
    heading.current?.focus();
  }, [skill]);

  const row = profileRow(skill, estimate);
  const name = t(`skill.${skill}`);
  const confidence =
    estimate?.confidence !== null && estimate?.confidence !== undefined && row.kind !== "insufficient" && row.kind !== "withheld"
      ? t(`skill.confidence.${confidenceBand(estimate.confidence)}`)
      : t("progress.none");

  function submit(event: FormEvent) {
    event.preventDefault();
    const value = parseAttemptNumber(text);
    setInvalid(value === null);
    setAttempt(value);
  }

  return (
    <Panel as="section" raised id="skill-drilldown" aria-labelledby="skill-drilldown-title" tone={skill}>
      <div className="px-stack">
        <div className="px-row">
          <h2 className="px-panel__title" id="skill-drilldown-title" tabIndex={-1} ref={heading}>
            {t("progress.drill.title", { skill: name })}
          </h2>
          <Button small onClick={onClose}>
            {t("common.close")}
          </Button>
        </div>
        <p className="px-hint">{t("progress.drill.rule")}</p>
        <dl className="facts">
          <div>
            <dt>{t("progress.drill.status")}</dt>
            <dd>{t(`progress.drill.status.${row.kind === "estimated" ? "estimated" : row.kind === "placement" ? "placement" : row.kind === "working_towards" ? "working" : row.kind}`)}</dd>
          </div>
          <div>
            <dt>{t("progress.drill.confidence")}</dt>
            <dd>{confidence}</dd>
          </div>
          <div>
            <dt>{t("progress.drill.tasks")}</dt>
            <dd>{row.tasks}</dd>
          </div>
          {estimate ? (
            <>
              <div>
                <dt>{t("progress.drill.version")}</dt>
                <dd className="phonetic facts__wrap">{estimate.algorithm_version}</dd>
              </div>
              <div>
                <dt>{t("progress.drill.computed")}</dt>
                <dd>{formatTime(estimate.computed_at)}</dd>
              </div>
            </>
          ) : null}
        </dl>

        <section className="px-stack" style={{ "--gap": "var(--space-3)" } as React.CSSProperties} aria-labelledby="skill-attempts-title">
          <h3 id="skill-attempts-title">{t("progress.drill.attempts.title")}</h3>
          <SkillAttempts skill={skill} onOpen={setAttempt} />
          <form className="px-stack" onSubmit={submit} noValidate>
            <TextField
              label={t("progress.lookup.label")}
              hint={t("progress.lookup.hint")}
              value={text}
              onChange={setText}
              inputMode="numeric"
              autoComplete="off"
              error={invalid ? t("progress.lookup.invalid") : undefined}
            />
            <div className="px-row">
              <Button type="submit" variant="primary">
                {t("progress.lookup.submit")}
              </Button>
            </div>
          </form>
          {attempt !== null ? <AttemptEvidencePanel key={attempt} attemptId={attempt} /> : null}
        </section>
      </div>
    </Panel>
  );
}
