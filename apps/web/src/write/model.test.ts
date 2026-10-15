import { describe, expect, it } from "vitest";
import type { DraftComparisonView } from "@/generated/DraftComparisonView";
import type { ErrorFindingView } from "@/generated/ErrorFindingView";
import type { ServerEvent } from "@/generated/ServerEvent";
import { draftFeedbackByTurn, markDraft, needsReview, revisionCounts, rubricSentence } from "./model";

function finding(quote: string, category: string, correction: string): ErrorFindingView {
  return { category, quote, correction, severity: "major", addressed_in_reply: false };
}

describe("draftFeedbackByTurn", () => {
  const draftEvent = (seq: number, session_id: number, turn_seq: number, status: "pending" | "analysed"): ServerEvent => ({
    type: "FeedbackReady",
    seq,
    session_id,
    feedback: {
      kind: "draft",
      feedback: { turn_seq, status, analysis: null, comparison: null, rubric: null },
    },
  });

  it("keys the feedback by turn_seq and keeps the newest one for a draft", () => {
    // Newest first, as the stream hands them over.
    const events: ServerEvent[] = [
      draftEvent(9, 42, 1, "analysed"),
      draftEvent(8, 42, 1, "pending"),
      draftEvent(7, 42, 2, "pending"),
    ];
    const found = draftFeedbackByTurn(events, 42);
    expect([...found.keys()].sort()).toEqual([1, 2]);
    expect(found.get(1)?.status).toBe("analysed");
  });

  it("ignores another session's feedback and anything that is not a draft", () => {
    const events: ServerEvent[] = [
      draftEvent(9, 7, 1, "analysed"),
      { type: "FeedbackReady", seq: 8, session_id: 42, feedback: { kind: "reading", score: { correct: 1, total: 2, score: 0.5, questions: [] } } },
    ];
    expect(draftFeedbackByTurn(events, 42).size).toBe(0);
  });
});

describe("revisionCounts", () => {
  it("counts fixed and remaining earlier errors apart, and the new ones", () => {
    const comparison: DraftComparisonView = {
      earlier: [
        { error: { quote: "I go", category: "verb_tense" }, resolution: "fixed" },
        { error: { quote: "she have", category: "agreement" }, resolution: "remaining" },
      ],
      new: [{ quote: "in monday", category: "preposition" }],
    };
    expect(revisionCounts(comparison)).toEqual({ fixed: 1, remaining: 1, added: 1 });
  });
});

describe("markDraft", () => {
  it("marks the server's quotes on the learner's own words, in order", () => {
    const text = "Yesterday I go to the market with my friends is kind.";
    const marked = markDraft(text, [finding("I go", "verb_tense", "I went"), finding("friends is", "agreement", "friends are")]);
    expect(marked.segments.map((segment) => segment.text).join("")).toBe(text);
    const marks = marked.segments.filter((segment) => segment.error !== undefined);
    expect(marks).toHaveLength(2);
    expect(marks[0]?.error).toEqual({ id: "e0", label: "verb_tense", correction: "I went" });
    expect(marks[1]?.error).toEqual({ id: "e1", label: "agreement", correction: "friends are" });
    expect(marked.unplaced).toEqual([]);
  });

  it("never invents an explanation, and never marks a quote twice", () => {
    const text = "I go, so I go again.";
    const marked = markDraft(text, [finding("I go", "verb_tense", "I went"), finding("I go", "verb_tense", "I went")]);
    const marks = marked.segments.filter((segment) => segment.error !== undefined);
    expect(marks).toHaveLength(2);
    expect(marks.map((segment) => segment.text)).toEqual(["I go", "I go"]);
    for (const mark of marks) expect(mark.error?.explanation).toBeUndefined();
  });

  it("returns a quote the draft does not contain instead of dropping it", () => {
    const marked = markDraft("A short draft.", [finding("not in the draft", "word_order", "better words")]);
    expect(marked.segments).toEqual([{ text: "A short draft." }]);
    expect(marked.unplaced).toHaveLength(1);
    expect(marked.unplaced[0]?.quote).toBe("not in the draft");
  });

  it("finds a quote whose spacing the server trimmed, and one with no errors at all", () => {
    const marked = markDraft("He have two brothers.", [finding(" He have ", "agreement", "He has")]);
    expect(marked.segments.filter((segment) => segment.error !== undefined)).toHaveLength(1);
    expect(markDraft("A clean draft.", []).segments).toEqual([{ text: "A clean draft." }]);
  });
});

describe("rubric helpers", () => {
  it("takes the sentence in the learner's language, and the other one when it is empty", () => {
    const rubric = { feedback_en: "English sentence.", feedback_l1: "Kalimat Indonesia." };
    expect(rubricSentence(rubric, "en")).toBe("English sentence.");
    expect(rubricSentence(rubric, "id")).toBe("Kalimat Indonesia.");
    expect(rubricSentence({ feedback_en: "English sentence.", feedback_l1: "  " }, "id")).toBe("English sentence.");
  });

  it("reads a dimension with no band, or one the server flagged, as needing review", () => {
    expect(needsReview({ band: null, status: "scored" })).toBe(true);
    expect(needsReview({ band: 3, status: "needs_review" })).toBe(true);
    expect(needsReview({ band: 3, status: "scored" })).toBe(false);
  });
});
