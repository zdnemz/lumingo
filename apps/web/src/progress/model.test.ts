import { describe, expect, it } from "vitest";
import type { SessionKind } from "@/generated/SessionKind";
import type { SkillEstimateView } from "@/generated/SkillEstimateView";
import { confidenceBand, parseProvenance, profileRow, sessionRole, unitIdOf } from "./model";

function estimate(patch: Partial<SkillEstimateView>): SkillEstimateView {
  return {
    skill: "speaking",
    level: "A2",
    status: "estimated",
    confidence: 0.7,
    evidence_count: 14,
    algorithm_version: "est/1",
    computed_at: "2026-10-06T10:00:00Z",
    ...patch,
  };
}

describe("confidenceBand", () => {
  it.each([
    [0.1, "low"],
    [0.59, "low"],
    [0.6, "medium"],
    [0.75, "medium"],
    [0.7501, "high"],
    [0.9, "high"],
  ] as const)("%s is %s", (value, band) => {
    expect(confidenceBand(value)).toBe(band);
  });
});

describe("profileRow", () => {
  it("shows a level only with its confidence band and its evidence count", () => {
    expect(profileRow("speaking", estimate({}))).toEqual({ kind: "estimated", skill: "speaking", level: "A2", band: "medium", tasks: 14 });
  });

  it("has no level for a skill the server has no row for", () => {
    expect(profileRow("writing", undefined)).toEqual({ kind: "insufficient", skill: "writing", tasks: 0 });
  });

  it("drops a level that arrives with the status insufficient_evidence", () => {
    const row = profileRow("writing", estimate({ status: "insufficient_evidence", level: "B1", confidence: 0.9, evidence_count: 3 }));
    expect(row).toEqual({ kind: "insufficient", skill: "writing", tasks: 3 });
  });

  it("drops a status of estimated that has no level", () => {
    expect(profileRow("reading", estimate({ level: null })).kind).toBe("insufficient");
  });

  it("withholds a level that lacks a confidence or an evidence count", () => {
    expect(profileRow("reading", estimate({ confidence: null })).kind).toBe("withheld");
    expect(profileRow("reading", estimate({ confidence: Number.NaN })).kind).toBe("withheld");
    expect(profileRow("reading", estimate({ evidence_count: 0 })).kind).toBe("withheld");
  });

  it("marks a placement-only level as a suggestion", () => {
    expect(profileRow("listening", estimate({ status: "placement_only", level: "A2", confidence: 0.5, evidence_count: 4 }))).toEqual({
      kind: "placement",
      skill: "listening",
      level: "A2",
      band: "low",
      tasks: 4,
    });
  });

  it("reads pre-A1 as working towards A1, never as a level", () => {
    expect(profileRow("listening", estimate({ level: "pre-A1", confidence: 0.4, evidence_count: 9 }))).toEqual({
      kind: "working_towards",
      skill: "listening",
      band: "low",
      tasks: 9,
    });
  });

  it("clamps a negative count to zero", () => {
    expect(profileRow("reading", estimate({ status: "insufficient_evidence", level: null, evidence_count: -2 }))).toMatchObject({ tasks: 0 });
  });
});

describe("sessionRole", () => {
  const roles: Record<SessionKind, string> = {
    lesson: "can_count",
    checkpoint: "can_count",
    placement: "can_count",
    conversation: "free",
    text_chat: "free",
    writing: "free",
    reading: "free",
    drill: "practice",
    review: "practice",
  };
  it.each(Object.entries(roles))("%s is %s", (kind, role) => {
    expect(sessionRole(kind as SessionKind)).toBe(role);
  });
});

describe("unitIdOf", () => {
  it("cuts at the first slash", () => {
    expect(unitIdOf("a1-u01/v3")).toBe("a1-u01");
    expect(unitIdOf("a1-u01/obj/1")).toBe("a1-u01");
    expect(unitIdOf("loose")).toBe("loose");
  });
});

describe("parseProvenance", () => {
  it("reads the metric row the recorder writes", () => {
    const parsed = parseProvenance({
      recorder: "evidence/1",
      scorer: "rubric_llm",
      scorer_version: "w1/1+t3/2+model-x",
      norm_version: "norm/1",
      algorithm_version: "rubric/1",
      engines: [{ role: "stt", id: "whisper-small", version: "1.2", model_checksum: "abc" }, { role: "x" }],
      details: { rubric: "w1/1", runs: 2, repaired: false, secret_note: "hidden", nested: { a: 1 } },
    });
    expect(parsed).toEqual({
      recorder: "evidence/1",
      scorer: "rubric_llm",
      scorerVersion: "w1/1+t3/2+model-x",
      normVersion: "norm/1",
      algorithmVersion: "rubric/1",
      engines: [{ role: "stt", id: "whisper-small", version: "1.2" }],
      details: [
        ["rubric", "w1/1"],
        ["runs", "2"],
        ["repaired", "false"],
      ],
    });
  });

  it("gives null for data that is not provenance", () => {
    expect(parseProvenance(null)).toBeNull();
    expect(parseProvenance("text")).toBeNull();
    expect(parseProvenance([1, 2])).toBeNull();
    expect(parseProvenance({ run_bands: [2, 3] })).toBeNull();
  });
});
