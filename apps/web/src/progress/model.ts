// Pure rules for what the progress screens may say. They turn what the server
// stored into one of a few allowed shapes (docs/ASSESSMENT_SPEC.md, sections 9
// and 12) and never compute a level, a score or a confidence themselves.

import type { EstimateLevel } from "@/generated/EstimateLevel";
import type { SessionKind } from "@/generated/SessionKind";
import type { SkillEstimateView } from "@/generated/SkillEstimateView";
import type { JsonValue } from "@/generated/serde_json/JsonValue";

/** The four skills that carry a CEFR-labelled estimate, in the order the profile shows them. */
export const PROFILE_SKILLS = ["listening", "speaking", "reading", "writing"] as const;
export type ProfileSkill = (typeof PROFILE_SKILLS)[number];

export type ConfidenceBand = "low" | "medium" | "high";

/** Display words from section 9.3: below 0.6 low, 0.6 to 0.75 medium, above 0.75 high. */
export function confidenceBand(confidence: number): ConfidenceBand {
  if (confidence < 0.6) return "low";
  if (confidence <= 0.75) return "medium";
  return "high";
}

/** The levels a learner can be estimated at, in learning order. */
export const LADDER = ["A1", "A2", "B1", "B2", "C1", "C2"] as const;
export type LadderLevel = (typeof LADDER)[number];

export type ProfileRow =
  /** A level secured by real attempts, shown with its confidence and evidence count. */
  | { kind: "estimated"; skill: ProfileSkill; level: LadderLevel; band: ConfidenceBand; tasks: number }
  /** A level that rests on the placement test alone: a suggested start, not a result. */
  | { kind: "placement"; skill: ProfileSkill; level: LadderLevel; band: ConfidenceBand; tasks: number }
  /** Below A1 so far. */
  | { kind: "working_towards"; skill: ProfileSkill; band: ConfidenceBand; tasks: number }
  /** The rule for enough evidence is not met. No level is shown. */
  | { kind: "insufficient"; skill: ProfileSkill; tasks: number }
  /** The server sent a level without the confidence or count that must travel with it, so it is not shown. */
  | { kind: "withheld"; skill: ProfileSkill; tasks: number };

function isLadderLevel(level: EstimateLevel): level is LadderLevel {
  return (LADDER as readonly string[]).includes(level);
}

/**
 * The row for one skill. A level appears only when its status says so and its
 * confidence and evidence count are both present; every other case shows no
 * level. A skill the server has no row for is "insufficient" with zero tasks.
 */
export function profileRow(skill: ProfileSkill, estimate: SkillEstimateView | undefined): ProfileRow {
  if (estimate === undefined) return { kind: "insufficient", skill, tasks: 0 };
  const tasks = Math.max(0, Math.trunc(estimate.evidence_count));
  if (estimate.status === "insufficient_evidence" || estimate.level === null) {
    return { kind: "insufficient", skill, tasks };
  }
  const confidence = estimate.confidence;
  if (confidence === null || !Number.isFinite(confidence) || tasks === 0) {
    return { kind: "withheld", skill, tasks };
  }
  const band = confidenceBand(confidence);
  if (estimate.level === "pre-A1") return { kind: "working_towards", skill, band, tasks };
  if (!isLadderLevel(estimate.level)) return { kind: "withheld", skill, tasks };
  return estimate.status === "placement_only"
    ? { kind: "placement", skill, level: estimate.level, band, tasks }
    : { kind: "estimated", skill, level: estimate.level, band, tasks };
}

export function isProfileSkill(skill: string): skill is ProfileSkill {
  return (PROFILE_SKILLS as readonly string[]).includes(skill);
}

/** How a kind of session relates to an estimate. */
export type SessionRole =
  /** Authored unit, checkpoint and placement work can add evidence. */
  | "can_count"
  /** Free modes never count toward a level (section 1, rule 6). */
  | "free"
  /** Drills and reviews are practice and are not used for a level. */
  | "practice";

/**
 * Only authored lesson, checkpoint and placement sessions can add evidence to
 * an estimate. Text chat, conversation, writing workshop and graded reading
 * are free modes. Drills and reviews are kept apart from a level as well.
 */
export function sessionRole(kind: SessionKind): SessionRole {
  switch (kind) {
    case "lesson":
    case "checkpoint":
    case "placement":
      return "can_count";
    case "conversation":
    case "text_chat":
    case "writing":
    case "reading":
      return "free";
    case "drill":
    case "review":
      return "practice";
  }
}

/** `<unit id>/<id>` to its unit id. A reference without a slash is its own unit id. */
export function unitIdOf(reference: string): string {
  const slash = reference.indexOf("/");
  return slash < 0 ? reference : reference.slice(0, slash);
}

export interface EngineStampView {
  role: string;
  id: string;
  version: string;
}

/** What the evidence recorder stores in a metric row so a score can be traced. */
export interface Provenance {
  recorder: string | null;
  scorer: string | null;
  scorerVersion: string | null;
  normVersion: string | null;
  algorithmVersion: string | null;
  engines: EngineStampView[];
  /** Plain values from the scorer's details, for the keys a learner or reviewer can use. */
  details: Array<[string, string]>;
}

const DETAIL_KEYS: readonly string[] = [
  "rubric",
  "contract_version",
  "model",
  "input_mode",
  "runs",
  "runs_agreed",
  "repaired",
  "ladder_level",
  "mode",
  "experimental",
];

function asRecord(value: JsonValue | undefined): Record<string, JsonValue> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value) ? value : null;
}

function text(value: JsonValue | undefined): string | null {
  return typeof value === "string" && value !== "" ? value : null;
}

/** Reads a metric row's `data`. Anything that does not look like provenance gives `null`. */
export function parseProvenance(data: JsonValue | null): Provenance | null {
  const record = asRecord(data ?? undefined);
  if (record === null) return null;
  const scorer = text(record.scorer);
  const scorerVersion = text(record.scorer_version);
  const normVersion = text(record.norm_version);
  const algorithmVersion = text(record.algorithm_version);
  if (scorer === null && scorerVersion === null && algorithmVersion === null && normVersion === null) return null;
  const engines: EngineStampView[] = [];
  if (Array.isArray(record.engines)) {
    for (const entry of record.engines) {
      const stamp = asRecord(entry);
      const id = text(stamp?.id);
      if (stamp === null || id === null) continue;
      engines.push({ role: text(stamp.role) ?? "", id, version: text(stamp.version) ?? "" });
    }
  }
  const details: Array<[string, string]> = [];
  const found = asRecord(record.details);
  if (found !== null) {
    for (const key of DETAIL_KEYS) {
      const value = found[key];
      if (typeof value === "string" || typeof value === "number" || typeof value === "boolean") {
        details.push([key, String(value)]);
      }
    }
  }
  return { recorder: text(record.recorder), scorer, scorerVersion, normVersion, algorithmVersion, engines, details };
}
