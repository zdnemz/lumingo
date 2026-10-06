// Data for the progress and game tests. Shapes are the generated API types, so
// a change on the server side breaks these at compile time.

import type { AttemptEvidence } from "@/generated/AttemptEvidence";
import type { GameState } from "@/generated/GameState";
import type { ProgressOverview } from "@/generated/ProgressOverview";
import type { SessionKind } from "@/generated/SessionKind";
import type { SkillEstimateView } from "@/generated/SkillEstimateView";
import type { UnitList } from "@/generated/UnitList";

export function estimateOf(patch: Partial<SkillEstimateView> & Pick<SkillEstimateView, "skill">): SkillEstimateView {
  return {
    level: "A2",
    status: "estimated",
    confidence: 0.7,
    evidence_count: 14,
    algorithm_version: "est/1",
    computed_at: "2026-10-06T10:00:00Z",
    ...patch,
  };
}

export const EMPTY_PROGRESS: ProgressOverview = {
  units: [],
  objectives: [],
  errors: [],
  reviews_due: [],
  reviews_due_truncated: false,
  estimates: [],
  recent_sessions: [],
};

export const FULL_PROGRESS: ProgressOverview = {
  units: [
    { unit_id: "a1-u01", status: "passed", best_checkpoint: 0.82, updated_at: "2026-10-05T10:00:00Z" },
    { unit_id: "a1-u02", status: "in_progress", best_checkpoint: null, updated_at: "2026-10-06T10:00:00Z" },
  ],
  objectives: [],
  errors: [
    { category: "grammar.article", count: 3, last_seen: "2026-10-05T09:00:00Z" },
    { category: "grammar.tense", count: 9, last_seen: "2026-10-06T09:00:00Z" },
    { category: "spelling", count: 3, last_seen: null },
  ],
  reviews_due: [
    { item_kind: "vocab", item_ref: "a1-u01/v3", due_at: "2026-10-04T08:00:00Z", interval_days: 2.5, reps: 3, lapses: 1 },
    { item_kind: "pron", item_ref: "a1-u02/p1", due_at: "2026-10-05T08:00:00Z", interval_days: 1, reps: 1, lapses: 0 },
  ],
  reviews_due_truncated: true,
  estimates: [
    estimateOf({ skill: "speaking", level: "A2", confidence: 0.7, evidence_count: 14 }),
    estimateOf({ skill: "reading", level: "B1", confidence: 0.82, evidence_count: 20 }),
    estimateOf({ skill: "listening", level: "A1", status: "placement_only", confidence: 0.5, evidence_count: 4 }),
    // A level that arrives with the status "insufficient evidence" must not be drawn.
    estimateOf({ skill: "writing", level: "B2", status: "insufficient_evidence", confidence: 0.9, evidence_count: 2 }),
  ],
  recent_sessions: [
    { id: 11, kind: "lesson", unit_id: "a1-u01", status: "completed", started_at: "2026-10-06T08:00:00Z", ended_at: "2026-10-06T08:20:00Z" },
    { id: 12, kind: "conversation", unit_id: null, status: "completed", started_at: "2026-10-06T09:00:00Z", ended_at: null },
    { id: 13, kind: "text_chat", unit_id: null, status: "aborted", started_at: "2026-10-06T09:30:00Z", ended_at: null },
    { id: 14, kind: "writing", unit_id: null, status: "completed", started_at: "2026-10-06T10:00:00Z", ended_at: null },
    { id: 15, kind: "reading", unit_id: null, status: "completed", started_at: "2026-10-06T10:30:00Z", ended_at: null },
    { id: 16, kind: "drill", unit_id: "a1-u02", status: "completed", started_at: "2026-10-06T11:00:00Z", ended_at: null },
    { id: 17, kind: "checkpoint", unit_id: "a1-u01", status: "completed", started_at: "2026-10-06T11:30:00Z", ended_at: null },
  ],
};

/** Every session kind there is, for tests that must cover them all. */
export const FREE_KINDS: readonly SessionKind[] = ["conversation", "text_chat", "writing", "reading"];

export const UNITS: UnitList = {
  content_version: "v1",
  issues: [],
  units: [
    {
      id: "a1-u01",
      level: "A1",
      sequence: 1,
      title: { en: "Hello, I am Sari", id: "Halo, aku Sari" },
      theme: "intro",
      estimated_minutes: 30,
      skill_counts: { listening: 1, speaking: 1, reading: 1, writing: 1, mediation: 0, grammar: 1, vocabulary: 1, pronunciation: 1 },
      objective_count: 4,
      prerequisites: [],
    },
  ],
};

export const GAME: GameState = {
  xp_total: 340,
  xp_by_source: [
    { source: "lesson", amount: 200 },
    { source: "text_chat", amount: 140 },
  ],
  rank: { rank: 3, progress_percent: 40, next_threshold: 500 },
  streak: { current: 4, longest: 9, active_today: true, rest_tokens_available: 0, tokens_needed_to_continue: 0 },
  streak_days: [{ date: "2026-10-06", counted: true }],
  cosmetics: [
    { id: "accessory-cap", kind: "accessory", rank: 1, unlocked: true, unlocked_at: "2026-10-01T00:00:00Z", equipped: true },
    { id: "theme-forest", kind: "theme", rank: 2, unlocked: true, unlocked_at: "2026-10-02T00:00:00Z", equipped: false },
    { id: "accessory-crown", kind: "accessory", rank: 5, unlocked: false, unlocked_at: null, equipped: false },
    { id: "accessory-new-thing", kind: "accessory", rank: 6, unlocked: false, unlocked_at: null, equipped: false },
  ],
  equipped: { accessory: "accessory-cap", theme: null },
};

export const EVIDENCE: AttemptEvidence = {
  attempt_id: 12,
  activity_id: "a1-u01/guided-writing-1",
  skill: "writing",
  dimension: "accuracy",
  evidence: [
    { id: 1, kind: "response_text", content: "I am from Jakarta.\nI like <b>tea</b>.", data: null, created_at: "2026-10-06T09:00:00Z" },
    { id: 2, kind: "quote", content: "I am from Jakarta", data: null, created_at: "2026-10-06T09:00:01Z" },
    { id: 3, kind: "scorer_reason", content: "The sentence is complete and correct.", data: null, created_at: "2026-10-06T09:00:02Z" },
    {
      id: 4,
      kind: "metric",
      content: null,
      data: {
        recorder: "evidence/1",
        scorer: "rubric_llm",
        scorer_version: "writing-a1/1+t3/2+model-x",
        norm_version: "norm/1",
        algorithm_version: "rubric/1",
        engines: [{ role: "stt", id: "whisper-small", version: "1.2", model_checksum: "abc" }],
        details: { rubric: "writing-a1/1", runs: 2, runs_agreed: true },
      },
      created_at: "2026-10-06T09:00:03Z",
    },
  ],
};
