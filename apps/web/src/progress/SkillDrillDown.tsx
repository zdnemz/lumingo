"use client";

import { useEffect, useRef, useState, type FormEvent } from "react";
import type { SkillEstimateView } from "@/generated/SkillEstimateView";
import { useFormatTime } from "@/providers/ProviderCard";
import { useT } from "@/state/PreferencesProvider";
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

export interface SkillDrillDownProps {
  skill: ProfileSkill;
  estimate: SkillEstimateView | undefined;
  onClose: () => void;
}

/**
 * From one skill's estimate to the work behind it. The facts come from the
 * estimate row (status, count, rule version). The server has no route that
 * lists the attempts of a skill, so that list says it is not available and the
 * only way in is one attempt number at a time.
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
          <div className="unavailable" role="note">
            <p className="px-hint">{t("progress.drill.attempts.unavailable")}</p>
          </div>
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
