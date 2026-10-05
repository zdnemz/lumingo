"use client";

import { Sprite } from "@/sprites/Sprite";
import type { SpriteName } from "@/sprites/data";
import { useT } from "@/state/PreferencesProvider";
import { Badge } from "@/ui/Badge";
import { Meter } from "@/ui/Meter";

export type Skill = "listening" | "speaking" | "reading" | "writing";
export type Confidence = "low" | "medium" | "high";

const LEVELS = ["A1", "A2", "B1", "B2", "C1", "C2"] as const;

export interface SkillEstimate {
  skill: Skill;
  /**
   * `estimate` carries a level and a confidence. `insufficient` means the rule
   * for "enough evidence" is not met yet, and then no level is shown at all.
   */
  status: "estimate" | "insufficient";
  level?: (typeof LEVELS)[number];
  confidence?: Confidence;
  scoredTasks: number;
  sessions: number;
  /** For `insufficient`: how many more tasks of this skill would help. */
  moreTasksNeeded?: number;
}

const ICONS: Record<Skill, SpriteName> = {
  listening: "skill-listening",
  speaking: "skill-speaking",
  reading: "skill-reading",
  writing: "skill-writing",
};

function levelPercent(level: SkillEstimate["level"]): number {
  const index = LEVELS.findIndex((value) => value === level);
  return index < 0 ? 0 : ((index + 1) / LEVELS.length) * 100;
}

/**
 * The four-skill profile. A level appears only inside the sentence that calls
 * it an estimate, and never without evidence behind it.
 */
export function SkillBars({ estimates }: { estimates: readonly SkillEstimate[] }) {
  const t = useT();
  return (
    <ul className="skills" aria-label={t("skill.profile")}>
      {estimates.map((entry) => {
        const skill = t(`skill.${entry.skill}`);
        const known = entry.status === "estimate" && entry.level && entry.confidence;
        const sentence = known
          ? t("skill.estimate", {
              skill,
              level: entry.level ?? "",
              confidence: t(`skill.confidence.${entry.confidence ?? "low"}`),
              tasks: entry.scoredTasks,
              sessions: entry.sessions,
            })
          : t("skill.insufficient", { skill: skill.toLowerCase(), more: entry.moreTasksNeeded ?? 1 });
        return (
          <li key={entry.skill} className="skills__row">
            <span className="skills__name" style={{ color: `var(--color-skill-${entry.skill})` }}>
              <Sprite name={ICONS[entry.skill]} scale={3} />
              <span>{skill}</span>
            </span>
            <Meter
              value={known ? levelPercent(entry.level) : 0}
              label={skill}
              tone={entry.skill}
            />
            <p className="skills__sentence">{sentence}</p>
            <Badge tone="estimate">{t("common.estimate")}</Badge>
          </li>
        );
      })}
    </ul>
  );
}
