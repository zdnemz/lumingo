"use client";

import type { Activity } from "@/generated/Activity";
import type { Localized } from "@/generated/Localized";
import type { Unit } from "@/generated/Unit";
import { usePreferences, useT } from "@/state/PreferencesProvider";
import { Badge } from "@/ui/Badge";
import { Panel } from "@/ui/Panel";
import { fourSkillOf, localized, type FourSkills } from "./model";

/** The four skills of the unit, in the order the profile shows them. */
const SKILLS: readonly FourSkills[] = ["listening", "speaking", "reading", "writing"];

/** The skill of an activity as one of the four. `null` for grammar, vocabulary and mediation. */
function skillOf(activity: Activity): FourSkills | null {
  return fourSkillOf(activity.skill);
}

/** Text in the learner's language, plus the unit's Indonesian when the help is on. */
function Text({ value, help }: { value: Localized; help: boolean }) {
  const { display } = usePreferences();
  return (
    <>
      {localized(value, display.language)}
      {help && display.language === "en" && value.id ? (
        <span lang="id" className="unit-help">
          {" "}
          {value.id}
        </span>
      ) : null}
    </>
  );
}

/**
 * What the unit teaches, shown beside the run: the authored presentation blocks
 * and the model dialogue as text, and the four skill sections with what this run
 * has answered so far.
 *
 * The per-skill totals are the unit file's own activities, and the answered
 * count is the set of activity ids the server accepted an answer for, so the
 * counts can only be as right as the run is. The Indonesian text is the unit's
 * own `id` text, shown when the help toggle is on.
 */
export function UnitIntro({
  unit,
  answeredIds,
  help,
}: {
  unit: Unit;
  answeredIds: ReadonlySet<string>;
  help: boolean;
}) {
  const t = useT();
  const perSkill = SKILLS.map((skill) => {
    const activities = unit.activities.filter((activity) => skillOf(activity) === skill);
    return { skill, activities, answered: activities.filter((activity) => answeredIds.has(activity.id)).length };
  });

  return (
    <div className="px-stack" style={{ "--gap": "var(--space-5)" } as React.CSSProperties}>
      <Panel title={t("unit.objectives.title")} raised>
        <div className="px-stack" style={{ "--gap": "var(--space-3)" } as React.CSSProperties}>
          {unit.objectives.length > 0 ? (
            <ul className="unit-objectives">
              {unit.objectives.map((objective) => (
                <li key={objective.id}>
                  <Badge>{t(`skill.${fourSkillOf(objective.skill) ?? "speaking"}`)}</Badge> <Text value={objective.can_do} help={help} />
                </li>
              ))}
            </ul>
          ) : null}
        </div>
      </Panel>

      <Panel title={t("unit.skills.title")} raised>
        <ul className="unit-skills">
          {perSkill.map(({ skill, activities, answered }) => (
            <li key={skill} className="unit-skill" data-skill={skill}>
              <p className="unit-skill__name">{t(`skill.${skill}`)}</p>
              {activities.length === 0 ? (
                <p className="px-hint">{t("unit.skills.none")}</p>
              ) : (
                <>
                  <p className="unit-skill__progress">{t("unit.skills.progress", { answered, total: activities.length })}</p>
                  <ul className="unit-skill__list">
                    {activities.map((activity) => (
                      <li key={activity.id} data-answered={answeredIds.has(activity.id) ? "true" : undefined}>
                        {answeredIds.has(activity.id) ? <span aria-hidden="true">✓ </span> : null}
                        <Text value={activity.instructions} help={help} />
                      </li>
                    ))}
                  </ul>
                </>
              )}
            </li>
          ))}
        </ul>
      </Panel>

      {unit.presentation.length > 0 ? (
        <Panel title={t("unit.plan.title")} raised>
          <div className="px-stack" style={{ "--gap": "var(--space-4)" } as React.CSSProperties}>
            {unit.presentation.map((block) => (
              <article key={block.id} className="unit-block">
                <h3>{t(`unit.plan.${block.kind}`)}</h3>
                <p>
                  <Text value={block.text} help={help} />
                </p>
                {block.examples && block.examples.length > 0 ? (
                  <ul className="unit-block__examples">
                    {block.examples.map((example, index) => (
                      <li key={index}>
                        <Text value={example} help={help} />
                      </li>
                    ))}
                  </ul>
                ) : null}
              </article>
            ))}
          </div>
        </Panel>
      ) : null}

      {unit.dialogues.length > 0 ? (
        <Panel title={t("unit.dialogue.title")} raised>
          <div className="px-stack" style={{ "--gap": "var(--space-5)" } as React.CSSProperties}>
            {unit.dialogues.map((dialogue) => (
              <section key={dialogue.id} className="unit-dialogue">
                <h3>{dialogue.title}</h3>
                <p className="px-hint">
                  <Text value={dialogue.context} help={help} />
                </p>
                <ol className="unit-dialogue__turns">
                  {dialogue.turns.map((turn, index) => (
                    <li key={index}>
                      <span className="unit-dialogue__speaker">{turn.speaker}</span>
                      <span>{turn.text}</span>
                    </li>
                  ))}
                </ol>
              </section>
            ))}
          </div>
        </Panel>
      ) : null}
    </div>
  );
}
