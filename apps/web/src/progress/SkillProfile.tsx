"use client";

import type { SkillEstimateView } from "@/generated/SkillEstimateView";
import { Sprite } from "@/sprites/Sprite";
import type { SpriteName } from "@/sprites/data";
import { useT, type Translate } from "@/state/PreferencesProvider";
import { Badge } from "@/ui/Badge";
import { Button } from "@/ui/Button";
import { LADDER, PROFILE_SKILLS, profileRow, type ProfileRow, type ProfileSkill } from "./model";

const ICONS: Record<ProfileSkill, SpriteName> = {
  listening: "skill-listening",
  speaking: "skill-speaking",
  reading: "skill-reading",
  writing: "skill-writing",
};

/** "based on 1 scored task" or "based on N scored tasks". */
export function basisText(t: Translate, tasks: number): string {
  return tasks === 1 ? t("skill.basis.one") : t("skill.basis.many", { tasks });
}

/** "N scored tasks so far", used under a skill that has no level. */
export function countText(t: Translate, tasks: number): string {
  if (tasks <= 0) return t("skill.count.zero");
  return tasks === 1 ? t("skill.count.one") : t("skill.count.many", { tasks });
}

/**
 * The one sentence a profile row shows. A level appears only inside a
 * sentence that calls it an estimate (or a suggested starting level), with its
 * confidence word and the number of scored tasks behind it.
 */
export function rowSentence(t: Translate, row: ProfileRow): string {
  const skill = t(`skill.${row.skill}`);
  switch (row.kind) {
    case "estimated":
      return t("skill.estimate.basis", {
        skill,
        level: row.level,
        confidence: t(`skill.confidence.${row.band}`),
        basis: basisText(t, row.tasks),
      });
    case "placement":
      return t("skill.placement", {
        level: row.level,
        confidence: t(`skill.confidence.${row.band}`),
        basis: basisText(t, row.tasks),
      });
    case "working_towards":
      return t("skill.working", { confidence: t(`skill.confidence.${row.band}`), basis: basisText(t, row.tasks) });
    case "insufficient":
      return t("skill.insufficient.general", { skill: skill.toLowerCase() });
    case "withheld":
      return t("skill.withheld", { skill: skill.toLowerCase() });
  }
}

/** The six levels as a ladder with the estimated one marked. Decoration only: the sentence says the same in words. */
function Ladder({ current }: { current: string }) {
  return (
    <ol className="ladder" aria-hidden="true">
      {LADDER.map((level) => (
        <li key={level} className="ladder__step" data-current={level === current ? "true" : undefined}>
          {level}
        </li>
      ))}
    </ol>
  );
}

export interface SkillProfileProps {
  estimates: readonly SkillEstimateView[];
  /** The skill whose drill-down is open. */
  open: ProfileSkill | null;
  onOpen: (skill: ProfileSkill, trigger: HTMLButtonElement) => void;
}

/**
 * The four-skill profile of `GET /api/progress`. Each skill is estimated on its
 * own; there is no single overall level (ASSESSMENT_SPEC 9.4). Anything else
 * the server sends under another skill name, pronunciation included, is not
 * drawn here.
 */
export function SkillProfile({ estimates, open, onOpen }: SkillProfileProps) {
  const t = useT();
  return (
    <ul className="profile" aria-label={t("skill.profile")}>
      {PROFILE_SKILLS.map((skill) => {
        const estimate = estimates.find((entry) => entry.skill === skill);
        const row = profileRow(skill, estimate);
        const name = t(`skill.${skill}`);
        const level = row.kind === "estimated" || row.kind === "placement" ? row.level : null;
        return (
          <li key={skill} className="profile__row" data-state={row.kind}>
            <div className="profile__head">
              <h3 className="profile__name" style={{ color: `var(--color-skill-${skill})` }}>
                <Sprite name={ICONS[skill]} scale={3} />
                <span>{name}</span>
              </h3>
              <Badge tone="estimate">{t("common.estimate")}</Badge>
            </div>
            {level !== null ? <Ladder current={level} /> : null}
            <p className="profile__sentence">{rowSentence(t, row)}</p>
            {row.kind === "insufficient" ? <p className="px-hint">{countText(t, row.tasks)}</p> : null}
            <Button
              small
              aria-expanded={open === skill}
              aria-controls={open === skill ? "skill-drilldown" : undefined}
              onClick={(event) => onOpen(skill, event.currentTarget)}
            >
              {t("skill.evidence")}{" "}
              <span className="sr-only">({name})</span>
            </Button>
          </li>
        );
      })}
    </ul>
  );
}
