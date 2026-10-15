// The writing screen's data, kept out of the component so the joins the server
// makes asynchronous can be tested on their own.
//
// Nothing here invents feedback: the rule findings are the server's own
// sentences, an error is the server's quote with the server's category and
// correction, and a band is the server's number. What the server did not send
// is shown as missing, never as a verdict.

import type { DraftAccepted } from "@/generated/DraftAccepted";
import type { DraftComparisonView } from "@/generated/DraftComparisonView";
import type { DraftFeedbackView } from "@/generated/DraftFeedbackView";
import type { ErrorFindingView } from "@/generated/ErrorFindingView";
import type { Level } from "@/generated/Level";
import type { ServerEvent } from "@/generated/ServerEvent";
import type { MarkedError, Segment } from "./AnnotatedText";

/**
 * The CEFR levels in learning order, spelled as the generated `Level` union
 * spells them. They are data the server accepts, never dictionary keys.
 */
export const LEVELS = ["A1", "A2", "B1", "B2", "C1", "C2"] as const satisfies readonly Level[];

/** The levels are exactly the ones the server takes. */
export type WriteLevel = (typeof LEVELS)[number];

/** A stored draft of this session, with the layers of its feedback as they arrive. */
export interface DraftEntry {
  /** The position of the draft in the stored session: the key the server uses too. */
  turn_seq: number;
  /** What the server counted, not what the editor thinks. */
  accepted: DraftAccepted;
}

/**
 * The feedback of every draft of this session, keyed by `turn_seq`.
 *
 * The layers arrive detached from the HTTP answer, so the screen keys them by
 * the draft's position and joins them when it draws: a `FeedbackReady` that
 * arrives before the answer to `submitDraft` is kept and shown with it.
 * `events` is newest first, so the newest feedback for a draft wins.
 */
export function draftFeedbackByTurn(
  events: readonly ServerEvent[],
  sessionId: number,
): Map<number, DraftFeedbackView> {
  const found = new Map<number, DraftFeedbackView>();
  for (const event of events) {
    if (event.type !== "FeedbackReady" || event.session_id !== sessionId) continue;
    if (event.feedback.kind !== "draft") continue;
    const feedback = event.feedback.feedback;
    if (!found.has(feedback.turn_seq)) found.set(feedback.turn_seq, feedback);
  }
  return found;
}

/** Fixed, remaining and new counts of a comparison, as `RevisionSummary` takes them. */
export function revisionCounts(comparison: DraftComparisonView): {
  fixed: number;
  remaining: number;
  added: number;
} {
  return {
    fixed: comparison.earlier.filter((earlier) => earlier.resolution === "fixed").length,
    remaining: comparison.earlier.filter((earlier) => earlier.resolution === "remaining").length,
    added: comparison.new.length,
  };
}

/** The words of the draft a mark covers, with the server's category and correction. */
function marked(error: ErrorFindingView, index: number): MarkedError {
  return {
    id: `e${index}`,
    label: error.category,
    correction: error.correction,
  };
}

interface Span {
  start: number;
  end: number;
  error: ErrorFindingView;
}

interface Place {
  start: number;
  end: number;
}

function overlaps(place: Place, taken: readonly Span[]): boolean {
  return taken.some((other) => place.start < other.end && other.start < place.end);
}

/** Where a quote sits in the draft, or nothing when it is not in it. */
function locate(text: string, quote: string, taken: readonly Span[]): Place | null {
  const trimmed = quote.trim();
  const candidates = trimmed === quote ? [quote] : [quote, trimmed];
  for (const needle of candidates) {
    if (needle === "") continue;
    let from = 0;
    for (;;) {
      const at = text.indexOf(needle, from);
      if (at < 0) break;
      const place = { start: at, end: at + needle.length };
      if (!overlaps(place, taken)) return place;
      from = at + 1;
    }
  }
  return null;
}

export interface MarkedDraft {
  /** The draft, split so every quoted mistake is its own mark. */
  segments: Segment[];
  /** Errors whose quote is not in the draft: shown as text, never dropped. */
  unplaced: ErrorFindingView[];
}

/**
 * Marks the errors of an analysis in the draft they came from. A quote the
 * draft does not contain cannot be marked, so it is returned unplaced and the
 * screen shows it as text: the program's report is never silently thinned.
 */
export function markDraft(text: string, errors: readonly ErrorFindingView[]): MarkedDraft {
  const spans: Span[] = [];
  const unplaced: ErrorFindingView[] = [];
  for (const error of errors) {
    const span = locate(text, error.quote, spans);
    if (span === null) {
      unplaced.push(error);
      continue;
    }
    spans.push({ start: span.start, end: span.end, error });
  }
  spans.sort((a, b) => a.start - b.start);

  const segments: Segment[] = [];
  let cursor = 0;
  spans.forEach((span, index) => {
    if (span.start > cursor) segments.push({ text: text.slice(cursor, span.start) });
    segments.push({ text: text.slice(span.start, span.end), error: marked(span.error, index) });
    cursor = span.end;
  });
  if (cursor < text.length) segments.push({ text: text.slice(cursor) });
  return { segments, unplaced };
}

/** The rubric sentence in the language the learner reads, falling back to the other one. */
export function rubricSentence(rubric: { feedback_en: string; feedback_l1: string }, language: "en" | "id"): string {
  const wanted = language === "id" ? rubric.feedback_l1 : rubric.feedback_en;
  const other = language === "id" ? rubric.feedback_en : rubric.feedback_l1;
  return wanted.trim() !== "" ? wanted : other;
}

/** True when the program did not score a dimension and says so. */
export function needsReview(dimension: { band: number | null; status: string }): boolean {
  return dimension.band === null || dimension.status === "needs_review";
}
