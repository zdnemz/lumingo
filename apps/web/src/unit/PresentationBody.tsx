"use client";

import type { ActivityPresentation } from "@/generated/ActivityPresentation";
import type { Localized } from "@/generated/Localized";
import { usePreferences, useT } from "@/state/PreferencesProvider";
import { Panel } from "@/ui/Panel";
import { localized } from "./model";

function Text({ value }: { value: Localized }) {
  const { display } = usePreferences();
  return <>{localized(value, display.language)}</>;
}

/**
 * The bodies this screen has no control for, even when the run offers them. All
 * of them are spoken: a spoken production needs one recording of the learner, a
 * read-aloud and a shadowing run are scored from recordings, and a spoken
 * minimal pair needs one recording per pair. This build has no recorder, so the
 * engine's own answer for these is a transcript or a clip it never gets.
 *
 * Each is shown as what it is — the text to read, the dialogue to repeat, the
 * words to say, the task — and the screen says in words that it has no control
 * for it. Nothing here pretends to be one: no box to type a spoken answer into,
 * no button, no disabled radio.
 */
export function PresentationBody({ activity }: { activity: ActivityPresentation }) {
  const t = useT();
  const body = activity.body;
  return (
    <div className="px-stack" style={{ "--gap": "var(--space-3)" } as React.CSSProperties}>
      {body.kind === "read_aloud" ? (
        <>
          <p className="unit-sentence px-panel px-panel--inset">{body.text}</p>
          {body.focus_phonemes.length > 0 ? (
            <p className="px-hint">{t("unit.read.focus", { sounds: body.focus_phonemes.join(", ") })}</p>
          ) : null}
        </>
      ) : null}

      {body.kind === "shadowing" ? (
        <>
          <h3>{body.title}</h3>
          <ol className="unit-dialogue__turns">
            {body.lines.map((line, index) => (
              <li key={index}>
                {line.speaker ? <span className="unit-dialogue__speaker">{line.speaker}</span> : null}
                <span>{line.text}</span>
              </li>
            ))}
          </ol>
        </>
      ) : null}

      {body.kind === "minimal_pairs_say" ? (
        <ul className="unit-pairs">
          {body.pairs.map((pair, index) => (
            <li key={index} className="px-row">
              <span className="unit-pairs__word">{pair.a}</span>
              <span className="unit-pairs__word">{pair.b}</span>
              <span className="px-hint">{pair.focus}</span>
            </li>
          ))}
        </ul>
      ) : null}

      {body.kind === "production" ? (
        <>
          <p>
            <Text value={body.prompt} />
          </p>
          <ul className="unit-objectives">
            {body.content_points.map((point, index) => (
              <li key={index}>{point}</li>
            ))}
          </ul>
        </>
      ) : null}

      <Panel inset role="note">
        <p className="px-hint">{t("unavailable.noui", { feature: t("feature.speech") })}</p>
      </Panel>
    </div>
  );
}
