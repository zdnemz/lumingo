"use client";

import type { ConversationSummaryView } from "@/generated/ConversationSummaryView";
import { useT } from "@/state/PreferencesProvider";
import { Badge } from "@/ui/Badge";
import { Panel } from "@/ui/Panel";

/**
 * The end of a conversation: what the server counted, exactly as it sent it.
 * The category names are the server's own words (its analysis vocabulary), so
 * they are shown as they arrive and never re-worded here.
 */
export function ConversationSummary({ summary }: { summary: ConversationSummaryView }) {
  const t = useT();
  return (
    <Panel title={t("talk.summary.title")} tone="primary" raised>
      <div className="px-stack" style={{ "--gap": "var(--space-4)" } as React.CSSProperties}>
        <p>{t("talk.summary.turns", { count: summary.learner_turns })}</p>

        {summary.top_errors.length === 0 ? (
          <p className="px-hint">{t("talk.summary.noerrors")}</p>
        ) : (
          <section className="px-stack" style={{ "--gap": "var(--space-3)" } as React.CSSProperties}>
            <h3>{t("talk.summary.errors")}</h3>
            <ul className="talk-errors">
              {summary.top_errors.map((pattern, index) => (
                <li key={index} className="talk-errors__row px-panel px-panel--inset">
                  <p className="talk-errors__head">
                    <Badge tone="info">{pattern.category}</Badge>
                    <span>{t("talk.summary.errorcount", { count: pattern.count })}</span>
                  </p>
                  <p className="talk-errors__quote">{t("talk.summary.quote", { text: pattern.quote })}</p>
                  <p>{t("talk.summary.correction", { text: pattern.correction })}</p>
                </li>
              ))}
            </ul>
          </section>
        )}

        {summary.unanalysed_turns.length > 0 ? (
          <p className="px-hint">{t("talk.summary.unanalysed", { turns: summary.unanalysed_turns.join(", ") })}</p>
        ) : null}
        {summary.analysis_unreliable ? <p className="px-hint">{t("talk.summary.unreliable")}</p> : null}
      </div>
    </Panel>
  );
}
