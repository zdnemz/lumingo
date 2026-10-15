"use client";

import { useMemo, useState } from "react";
import type { DraftAccepted } from "@/generated/DraftAccepted";
import type { DraftFeedbackView } from "@/generated/DraftFeedbackView";
import type { RubricDimensionView } from "@/generated/RubricDimensionView";
import { usePreferences, useT } from "@/state/PreferencesProvider";
import { Badge } from "@/ui/Badge";
import { Panel } from "@/ui/Panel";
import { AnnotatedText } from "./AnnotatedText";
import { markDraft, needsReview, revisionCounts, rubricSentence } from "./model";
import { RevisionSummary } from "./RevisionSummary";

/** The rule checker's own sentences, exactly as the server wrote them. */
function RuleFindings({ accepted }: { accepted: DraftAccepted }) {
  const t = useT();
  if (!accepted.rule_checked) {
    // False means nothing was checked, so the screen must not read as "clean".
    return <p className="write__rules-state">{t("write.rules.notchecked")}</p>;
  }
  if (accepted.rule_findings.length === 0) {
    return <p className="write__rules-state">{t("write.rules.none")}</p>;
  }
  return (
    <ul className="write__findings">
      {accepted.rule_findings.map((finding, index) => (
        <li key={index}>{finding}</li>
      ))}
    </ul>
  );
}

/** One dimension of the rubric: its band, or that it needs a human look, and the reason why. */
function Band({ dimension }: { dimension: RubricDimensionView }) {
  const t = useT();
  const review = needsReview(dimension);
  return (
    <li className="px-stack write__band">
      <h4 className="write__band-name">{dimension.dimension}</h4>
      <p className="px-row">
        <Badge tone={review ? "estimate" : "info"}>
          {review ? t("write.band.review") : t("write.band", { band: dimension.band ?? "" })}
        </Badge>
        {dimension.capped ? <span className="px-hint">{t("write.band.capped")}</span> : null}
      </p>
      {dimension.reason.trim() !== "" ? <p className="write__band-reason">{dimension.reason}</p> : null}
      {dimension.evidence_quotes.length > 0 ? (
        <ul className="write__band-quotes" aria-label={t("write.rubric.evidence")}>
          {dimension.evidence_quotes.map((quote, index) => (
            <li key={index}>{quote}</li>
          ))}
        </ul>
      ) : null}
    </li>
  );
}

/** Layers two and three, in the state the server says they are in. */
function Feedback({ feedback, text }: { feedback: DraftFeedbackView; text: string }) {
  const t = useT();
  const { display } = usePreferences();
  const [selected, setSelected] = useState<string | null>(null);
  const errors = useMemo(() => feedback.analysis?.errors ?? [], [feedback.analysis]);
  const marked = useMemo(() => markDraft(text, errors), [text, errors]);

  if (feedback.status === "pending") {
    // Offline: the analysis failed, the draft is queued, and everything else is null.
    return (
      <>
        <h4>{t("write.feedback.pending.title")}</h4>
        <p>{t("write.feedback.pending.body")}</p>
      </>
    );
  }

  const nothing = feedback.analysis === null && feedback.rubric === null && feedback.comparison === null;

  return (
    <>
      {feedback.status === "rubric_pending" ? <p>{t("write.feedback.rubricpending")}</p> : null}
      {nothing ? <p>{t("write.feedback.empty")}</p> : null}

      {errors.length > 0 ? (
        <>
          <AnnotatedText segments={marked.segments} selectedId={selected} onSelect={setSelected} />
          {marked.unplaced.length > 0 ? (
            <div className="px-stack write__unplaced">
              <p>{t("write.errors.unplaced")}</p>
              <ul className="write__unplaced-list">
                {marked.unplaced.map((error, index) => (
                  <li key={index}>
                    <p className="annotated__label">{error.category}</p>
                    <p>{t("write.fix", { correction: error.correction })}</p>
                    {error.quote.trim() !== "" ? (
                      <p className="write__unplaced-quote">{t("write.quote", { quote: error.quote })}</p>
                    ) : null}
                  </li>
                ))}
              </ul>
            </div>
          ) : null}
        </>
      ) : null}

      {feedback.rubric !== null ? (
        <>
          <h4>{t("write.rubric.title")}</h4>
          <p className="px-hint">{t("write.rubric.hint")}</p>
          {feedback.rubric.on_task === false ? <p>{t("write.rubric.offtask")}</p> : null}
          <ul className="write__bands">
            {feedback.rubric.dimensions.map((dimension, index) => (
              <Band key={index} dimension={dimension} />
            ))}
          </ul>
          {rubricSentence(feedback.rubric, display.language).trim() !== "" ? (
            <p>{rubricSentence(feedback.rubric, display.language)}</p>
          ) : null}
        </>
      ) : null}

      {feedback.comparison !== null ? <RevisionSummary {...revisionCounts(feedback.comparison)} /> : null}
    </>
  );
}

export interface DraftPanelProps {
  /** What the server counted and found by rules when the draft was handed in. */
  accepted: DraftAccepted;
  /** The draft as it was handed in, so the marks land on the learner's own words. */
  text: string;
  /** Layers two and three, when the `FeedbackReady` event for this draft has arrived. */
  feedback: DraftFeedbackView | null;
}

/**
 * One draft: what the server counted and found by rules at once, and the rest
 * of the feedback when its detached event arrives. A missing layer is shown as
 * missing: there is no invented verdict and no invented correction.
 */
export function DraftPanel({ accepted, text, feedback }: DraftPanelProps) {
  const t = useT();
  return (
    <Panel tone="writing" raised className="px-stack write__draft">
      <p className="write__draft-number">{t("write.draft.number", { number: accepted.turn_seq })}</p>
      <p className="write__handed-in">{text}</p>
      <p className="write__words">{t("write.words", { count: accepted.words })}</p>
      {accepted.below_minimum ? <p className="px-hint">{t("write.below.minimum")}</p> : null}

      <section className="px-stack" style={{ "--gap": "var(--space-2)" } as React.CSSProperties}>
        <h3>{t("write.rules.title")}</h3>
        <RuleFindings accepted={accepted} />
      </section>

      <section className="px-stack" style={{ "--gap": "var(--space-2)" } as React.CSSProperties}>
        <h3>{t("write.feedback.title")}</h3>
        {feedback === null ? <p role="status">{t("state.working")}</p> : <Feedback feedback={feedback} text={text} />}
      </section>
    </Panel>
  );
}
