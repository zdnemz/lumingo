"use client";

import type { ActivityPresentation } from "@/generated/ActivityPresentation";
import type { Localized } from "@/generated/Localized";
import { GapFillActivity } from "@/activities/GapFillActivity";
import { MatchActivity } from "@/activities/MatchActivity";
import { McqActivity } from "@/activities/McqActivity";
import { ReorderActivity } from "@/activities/ReorderActivity";
import { TextAnswer } from "@/activities/TextAnswer";
import { usePreferences, useT } from "@/state/PreferencesProvider";
import { localized, type Draft } from "./model";

function Text({ value }: { value: Localized }) {
  const { display } = usePreferences();
  return <>{localized(value, display.language)}</>;
}

export interface ActivityBodyViewProps {
  activity: ActivityPresentation;
  /** What the learner typed or tapped so far. */
  draft: Draft;
  onChange: (patch: Partial<Draft>) => void;
  /** The choices lock once an answer was handed in. */
  locked?: boolean;
}

/**
 * One activity body, drawn with the components the activities folder already
 * has. The word range of a writing task comes from the body, which the server
 * sends with it, so the count can be shown without the unit file.
 *
 * Every body the run offers and this screen can answer is here. The bodies that
 * are played aloud (a listening item, a dictation, a listening minimal pair) are
 * here too: the program plays the sound on the learner's own computer, and the
 * screen shows the question and takes the pick.
 */
export function ActivityBodyView({ activity, draft, onChange, locked }: ActivityBodyViewProps) {
  const t = useT();
  const body = activity.body;
  switch (body.kind) {
    case "mcq":
      return (
        <McqActivity
          stem={body.stem}
          options={body.options}
          passage={body.passage ?? undefined}
          selected={draft.choice}
          locked={locked}
          onSelect={(index) => onChange({ choice: index })}
        />
      );
    case "gap_fill":
      return (
        <GapFillActivity
          text={body.text}
          answers={draft.gaps}
          locked={locked}
          onChange={(index, value) => {
            const gaps = [...draft.gaps];
            while (gaps.length < body.gaps) gaps.push("");
            gaps[index] = value;
            onChange({ gaps });
          }}
        />
      );
    case "reorder":
      return (
        <ReorderActivity
          tokens={body.tokens}
          placed={draft.placed}
          locked={locked}
          onPlace={(tokenIndex) => onChange({ placed: [...draft.placed, tokenIndex] })}
          onRemove={(position) => onChange({ placed: draft.placed.filter((_, index) => index !== position) })}
          onReset={() => onChange({ placed: [] })}
        />
      );
    case "match":
      return (
        <MatchActivity
          left={body.left}
          right={body.right}
          chosen={body.left.map((_, index) => draft.picks[index] ?? null)}
          locked={locked}
          onChoose={(leftIndex, rightIndex) => {
            const picks = [...draft.picks];
            while (picks.length < body.left.length) picks.push(null);
            picks[leftIndex] = rightIndex;
            onChange({ picks });
          }}
        />
      );
    case "dictation":
    case "error_correction":
      return (
        <div className="px-stack" style={{ "--gap": "var(--space-3)" } as React.CSSProperties}>
          {body.kind === "error_correction" ? (
            <p className="unit-sentence px-panel px-panel--inset">{body.sentence}</p>
          ) : null}
          <TextAnswer value={draft.text} onChange={(text) => onChange({ text })} locked={locked} />
        </div>
      );
    case "mediation":
      return (
        <div className="px-stack" style={{ "--gap": "var(--space-3)" } as React.CSSProperties}>
          <p className="unit-sentence px-panel px-panel--inset">{body.source_text}</p>
          <p>
            <Text value={body.task} />
          </p>
          <TextAnswer value={draft.text} onChange={(text) => onChange({ text })} rows={4} locked={locked} />
        </div>
      );
    case "production":
      return (
        <div className="px-stack" style={{ "--gap": "var(--space-3)" } as React.CSSProperties}>
          <p>
            <Text value={body.prompt} />
          </p>
          <ul className="unit-objectives">
            {body.content_points.map((point, index) => (
              <li key={index}>{point}</li>
            ))}
          </ul>
          <TextAnswer
            value={draft.text}
            onChange={(text) => onChange({ text })}
            rows={4}
            words={{ min: body.min_words, max: body.max_words }}
            locked={locked}
          />
        </div>
      );
    case "reading_set":
      return (
        <div className="px-stack" style={{ "--gap": "var(--space-3)" } as React.CSSProperties}>
          <p className="unit-sentence px-panel px-panel--inset">{body.passage}</p>
          {body.questions.map((question, index) => (
            <McqActivity
              key={index}
              stem={question.stem}
              options={question.options}
              selected={draft.picks[index] ?? null}
              locked={locked}
              onSelect={(option) => {
                const picks = [...draft.picks];
                while (picks.length < body.questions.length) picks.push(null);
                picks[index] = option;
                onChange({ picks });
              }}
            />
          ))}
        </div>
      );
    case "listening_set":
      return (
        <div className="px-stack" style={{ "--gap": "var(--space-3)" } as React.CSSProperties}>
          <h3>{t("unit.listen.questions")}</h3>
          {body.questions.map((question, index) => (
            <McqActivity
              key={index}
              stem={question.stem}
              options={question.options}
              selected={draft.picks[index] ?? null}
              locked={locked}
              onSelect={(option) => {
                const picks = [...draft.picks];
                while (picks.length < body.questions.length) picks.push(null);
                picks[index] = option;
                onChange({ picks });
              }}
            />
          ))}
        </div>
      );
    case "minimal_pairs_listen":
      return (
        <div className="px-stack" style={{ "--gap": "var(--space-3)" } as React.CSSProperties}>
          {body.items.map((item, index) => (
            <McqActivity
              key={index}
              stem={t("unit.listen.pairs")}
              options={item.options}
              selected={draft.picks[index] ?? null}
              locked={locked}
              onSelect={(option) => {
                const picks = [...draft.picks];
                while (picks.length < body.items.length) picks.push(null);
                picks[index] = option;
                onChange({ picks });
              }}
            />
          ))}
        </div>
      );
    default:
      // The spoken bodies and the roleplay are drawn by their own panels.
      return null;
  }
}
