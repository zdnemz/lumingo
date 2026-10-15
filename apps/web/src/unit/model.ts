// Pure rules for the unit screen. They read what the server sent — the unit
// list, the progress overview, the unit file, the run — and never compute a
// score, a level or an activity set of their own.

import type { ActivityAnswer } from "@/generated/ActivityAnswer";
import type { ActivityPresentation } from "@/generated/ActivityPresentation";
import type { L1HelpMode } from "@/generated/L1HelpMode";
import type { Level } from "@/generated/Level";
import type { Localized } from "@/generated/Localized";
import type { ProgressOverview } from "@/generated/ProgressOverview";
import type { Skill } from "@/generated/Skill";
import type { SkillCounts } from "@/generated/SkillCounts";
import type { Unit } from "@/generated/Unit";
import type { UnitSummary } from "@/generated/UnitSummary";
import type { UnitStatus } from "@/generated/UnitStatus";
import type { QuestNode, QuestRegion } from "@/learn/QuestMap";

/** The four skills the profile and this screen report on. */
export type FourSkills = "listening" | "speaking" | "reading" | "writing";

export const FOUR_SKILLS: readonly FourSkills[] = ["listening", "speaking", "reading", "writing"];

/**
 * Which of the four skills an activity trains. Both speaking skills and
 * pronunciation count as speaking, which is the same rule the server uses for
 * `skill_counts`. Grammar, vocabulary and mediation are targets of their own
 * and are not one of the four skills, so they get no section here.
 */
export function fourSkillOf(skill: Skill): FourSkills | null {
  switch (skill) {
    case "listening":
      return "listening";
    case "reading":
      return "reading";
    case "writing":
      return "writing";
    case "speaking_production":
    case "speaking_interaction":
    case "pronunciation":
      return "speaking";
    default:
      return null;
  }
}

/** How many of the four skills the unit has practice for, from the server's own counts. */
export function skillsPractised(counts: SkillCounts): number {
  return [counts.listening, counts.speaking, counts.reading, counts.writing].filter((count) => count > 0).length;
}

/** A localized text in the language the learner reads. */
export function localized(text: Localized, language: "en" | "id"): string {
  return language === "id" ? (text.id ?? text.en) : text.en;
}

/** The title of a unit in the language the learner reads. */
export function unitTitle(summary: UnitSummary, language: "en" | "id"): string {
  return localized(summary.title, language);
}

function isDone(status: UnitStatus | undefined): boolean {
  return status === "passed" || status === "skipped";
}

/**
 * The quest map, built from the unit list and the progress overview. The
 * mapping, and why it is the simplest honest one:
 *
 *   done    the unit's own progress row says `passed` or `skipped`. Nothing else
 *           counts as done: a run that was left unfinished is not a finished
 *           unit, and the list alone never says so.
 *   locked  the unit is not done and at least one of its own prerequisites is
 *           not done. A unit that declares no prerequisites is never locked, so
 *           a fresh install still offers the first unit.
 *   current the first unit whose progress row says `in_progress`; when no row
 *           says that, the first unit in learning order that is neither done nor
 *           locked.
 *   open    every other unit that is neither done nor locked.
 *
 * The list arrives in learning order (level, then sequence), so the regions keep
 * that order and a node's number is its position inside its level.
 */
export function questRegions(units: readonly UnitSummary[], overview: ProgressOverview, language: "en" | "id"): QuestRegion[] {
  const status = new Map<string, UnitStatus>();
  for (const row of overview.units) status.set(row.unit_id, row.status);

  const unlocked = (unit: UnitSummary): boolean => unit.prerequisites.every((id) => isDone(status.get(id)));
  const current =
    units.find((unit) => status.get(unit.id) === "in_progress") ??
    units.find((unit) => !isDone(status.get(unit.id)) && unlocked(unit));

  const byLevel = new Map<string, QuestNode[]>();
  for (const unit of units) {
    const state =
      isDone(status.get(unit.id)) ? "done" : !unlocked(unit) ? "locked" : unit.id === current?.id ? "current" : "open";
    const node: QuestNode = {
      id: unit.id,
      number: unit.sequence,
      title: unitTitle(unit, language),
      state,
      skillsPractised: skillsPractised(unit.skill_counts),
    };
    const nodes = byLevel.get(unit.level);
    if (nodes) nodes.push(node);
    else byLevel.set(unit.level, [node]);
  }
  return [...byLevel].map(([level, nodes]) => ({ level, nodes }));
}

/** How many activities of one of the four skills the run has answered, out of the unit's own count. */
export interface SkillProgress {
  skill: FourSkills;
  answered: number;
  total: number;
}

/**
 * Per-skill progress for the unit. The totals come from the unit file; the
 * answered counts come from the activity ids the server accepted an answer for
 * during this run, so an activity the run never reached is not counted.
 */
export function skillProgress(unit: Unit, answeredIds: ReadonlySet<string>): SkillProgress[] {
  return FOUR_SKILLS.map((skill) => {
    const ids = unit.activities.filter((activity) => fourSkillOf(activity.skill) === skill).map((activity) => activity.id);
    return { skill, answered: ids.filter((id) => answeredIds.has(id)).length, total: ids.length };
  });
}

/**
 * Whether the unit's Indonesian help starts switched on, from the learner's
 * setting: `on` always, `off` never, `auto` for A1 and A2 — which is what the
 * settings screen promises. Until the first snapshot arrives nothing is known,
 * so the help starts off.
 */
export function helpStartsOn(mode: L1HelpMode | null, level: Level): boolean {
  if (mode === "on") return true;
  if (mode === "off" || mode === null) return false;
  return level === "A1" || level === "A2";
}

/** What the learner has put into the activity on screen, before it is handed in. */
export interface Draft {
  /** The chosen option of an `mcq`. */
  choice: number | null;
  /** One answer per gap of a `gap_fill`. */
  gaps: string[];
  /** Positions in `tokens` of a `reorder`, in the order the learner placed them. */
  placed: number[];
  /** One pick per item: the questions of a reading or listening set, the pairs of a `match`, the words of a listening minimal pair. */
  picks: (number | null)[];
  /** Typed text: a dictation, a corrected sentence, a mediation, a written production. */
  text: string;
}

export function emptyDraft(): Draft {
  return { choice: null, gaps: [], placed: [], picks: [], text: "" };
}

export interface ReadyAnswer {
  /** What to hand in, or `null` while there is nothing the server would take. */
  answer: ActivityAnswer | null;
  /** The control can be pressed: the answer is complete enough for the server. */
  canSubmit: boolean;
  /**
   * This build has no control for the body at all: no button is drawn and the
   * screen says so, rather than sending an answer the server would refuse.
   */
  noControl: boolean;
}

function incomplete(): ReadyAnswer {
  return { answer: null, canSubmit: false, noControl: false };
}

function noControl(): ReadyAnswer {
  return { answer: null, canSubmit: false, noControl: true };
}

function ready(answer: ActivityAnswer): ReadyAnswer {
  return { answer, canSubmit: true, noControl: false };
}

function textAnswer(text: string): ReadyAnswer {
  return text.trim() === "" ? incomplete() : ready({ kind: "text", text });
}

function pickAnswer(picks: (number | null)[]): ReadyAnswer {
  return picks.length > 0 && picks.every((pick) => pick !== null) ? ready({ kind: "picks", picks }) : incomplete();
}

/**
 * The answer for the activity on screen, in the shape
 * `POST /api/activities/submit` documents for that body. It is built only from
 * what the learner typed or tapped: nothing is filled in for them, and a body
 * this screen cannot answer says so through `noControl` so no button is drawn.
 *
 * A read-aloud, a shadowing run and a spoken minimal pair are the bodies the
 * engine takes only as recorded clips of the learner. This build has no
 * recorder, so they get no control — and offline the run reports them as
 * unavailable before the screen would ever offer them. A spoken production is
 * the same: the engine wants a transcript from the recogniser.
 *
 * An item that is played aloud (a listening item, a dictation, a listening
 * minimal pair) is played by the program on the learner's own computer, never by
 * the browser, so the screen may show its questions and take the picks; offline
 * the run reports those activities as unavailable with its own reason.
 */
export function answerOf(presentation: ActivityPresentation, draft: Draft): ReadyAnswer {
  const body = presentation.body;
  switch (body.kind) {
    case "mcq":
      return draft.choice === null ? incomplete() : ready({ kind: "choice", index: draft.choice });
    case "gap_fill": {
      const answers = draft.gaps.slice(0, body.gaps);
      const complete = answers.length === body.gaps && answers.every((answer) => answer.trim() !== "");
      return complete ? ready({ kind: "gaps", answers }) : incomplete();
    }
    case "reorder":
      return draft.placed.length === 0
        ? incomplete()
        : ready({ kind: "order", tokens: draft.placed.map((index) => body.tokens[index] ?? "") });
    case "match":
      return pickAnswer(body.left.map((_, index) => draft.picks[index] ?? null));
    case "reading_set":
      return pickAnswer(body.questions.map((_, index) => draft.picks[index] ?? null));
    case "listening_set":
      return pickAnswer(body.questions.map((_, index) => draft.picks[index] ?? null));
    case "minimal_pairs_listen":
      return pickAnswer(body.items.map((_, index) => draft.picks[index] ?? null));
    case "dictation":
    case "error_correction":
    case "mediation":
      return textAnswer(draft.text);
    case "production":
      return presentation.activity_type === "guided_speaking" ? noControl() : textAnswer(draft.text);
    case "read_aloud":
    case "shadowing":
    case "minimal_pairs_say":
      // Scored from recorded clips of the learner, which this screen cannot make.
      return noControl();
    case "roleplay":
      // The roleplay is started and finished by its own panel, never by a check.
      return noControl();
  }
}

/** The instruction of an activity of the unit in the learner's language, for a list that carries only ids. */
export function instructionOf(unit: Unit, activityId: string, language: "en" | "id"): string {
  const activity = unit.activities.find((candidate) => candidate.id === activityId);
  return activity ? localized(activity.instructions, language) : activityId;
}
