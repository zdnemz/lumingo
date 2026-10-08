"use client";

import type { ErrorStatView } from "@/generated/ErrorStatView";
import type { ProgressOverview } from "@/generated/ProgressOverview";
import type { ReviewDueView } from "@/generated/ReviewDueView";
import type { SessionSummary } from "@/generated/SessionSummary";
import type { SkillEstimateView } from "@/generated/SkillEstimateView";
import type { UnitProgressView } from "@/generated/UnitProgressView";
import { useFormatTime } from "@/providers/ProviderCard";
import { useT } from "@/state/PreferencesProvider";
import { Badge } from "@/ui/Badge";
import { Panel } from "@/ui/Panel";
import { PROFILE_SKILLS, sessionRole, unitIdOf } from "./model";

/** What the program knows about each unit's name, by unit id. Empty when the unit list could not be read. */
export type UnitTitles = ReadonlyMap<string, string>;

function unitLabel(titles: UnitTitles, reference: string): string {
  const id = unitIdOf(reference);
  return titles.get(id) ?? id;
}

/**
 * Estimate history. The program keeps only the newest estimate of each skill
 * (`latest_per_skill`), so no curve can be drawn. This panel says so and lists
 * what is real: when each estimate was last worked out and by which rule.
 */
export function HistoryPanel({ estimates }: { estimates: readonly SkillEstimateView[] }) {
  const t = useT();
  const formatTime = useFormatTime();
  const known = PROFILE_SKILLS.flatMap((skill) => {
    const estimate = estimates.find((entry) => entry.skill === skill);
    return estimate ? [{ skill, estimate }] : [];
  });
  return (
    <Panel title={t("progress.history.title")} raised>
      <div className="px-stack" style={{ "--gap": "var(--space-3)" } as React.CSSProperties}>
        <div className="unavailable" role="note">
          <p className="px-hint">{t("progress.history.unavailable")}</p>
        </div>
        {known.length > 0 ? (
          <ul className="rows">
            {known.map(({ skill, estimate }) => (
              <li key={skill} className="rows__item">
                <span className="rows__text">
                  {t("progress.history.latest", {
                    skill: t(`skill.${skill}`),
                    time: formatTime(estimate.computed_at),
                    version: estimate.algorithm_version,
                  })}
                </span>
              </li>
            ))}
          </ul>
        ) : null}
      </div>
    </Panel>
  );
}

/** Error patterns from `error_stats`: counts per category, most frequent first. Not part of any estimate. */
export function ErrorPatterns({ errors }: { errors: readonly ErrorStatView[] }) {
  const t = useT();
  const formatTime = useFormatTime();
  const sorted = [...errors].sort((a, b) => b.count - a.count || a.category.localeCompare(b.category));
  return (
    <Panel title={t("progress.errors.title")} raised>
      <div className="px-stack" style={{ "--gap": "var(--space-3)" } as React.CSSProperties}>
        <p className="px-hint">{t("progress.errors.intro")}</p>
        {sorted.length === 0 ? (
          <p>{t("progress.errors.empty")}</p>
        ) : (
          <div className="table-wrap">
            <table className="table">
              <thead>
                <tr>
                  <th scope="col">{t("progress.errors.category")}</th>
                  <th scope="col">{t("progress.errors.count")}</th>
                  <th scope="col">{t("progress.errors.last")}</th>
                </tr>
              </thead>
              <tbody>
                {sorted.map((row) => (
                  <tr key={row.category}>
                    <th scope="row" className="phonetic">
                      {row.category}
                    </th>
                    <td>{row.count}</td>
                    <td>{row.last_seen ? formatTime(row.last_seen) : t("progress.none")}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>
    </Panel>
  );
}

function ReviewItem({ item, titles }: { item: ReviewDueView; titles: UnitTitles }) {
  const t = useT();
  const formatTime = useFormatTime();
  return (
    <li className="rows__item review">
      <div className="px-stack" style={{ "--gap": "var(--space-2)" } as React.CSSProperties}>
        <p className="px-row">
          <strong className="rows__text">{unitLabel(titles, item.item_ref)}</strong>
          <Badge tone={item.item_kind === "pron" ? "info" : undefined}>{t(`progress.review.kind.${item.item_kind}`)}</Badge>
        </p>
        <p className="phonetic review__ref">{item.item_ref}</p>
        <dl className="facts">
          <div>
            <dt>{t("progress.reviews.due")}</dt>
            <dd>{formatTime(item.due_at)}</dd>
          </div>
          <div>
            <dt>{t("progress.reviews.interval")}</dt>
            <dd>{Math.round(item.interval_days * 10) / 10}</dd>
          </div>
          <div>
            <dt>{t("progress.reviews.reps")}</dt>
            <dd>{item.reps}</dd>
          </div>
          <div>
            <dt>{t("progress.reviews.lapses")}</dt>
            <dd>{item.lapses}</dd>
          </div>
        </dl>
      </div>
    </li>
  );
}

/** Review items due now, from `reviews_due` of `GET /api/progress`. */
export function ReviewQueue({ overview, titles }: { overview: ProgressOverview; titles: UnitTitles }) {
  const t = useT();
  return (
    <Panel title={t("progress.reviews.title")} raised>
      <div className="px-stack" style={{ "--gap": "var(--space-3)" } as React.CSSProperties}>
        <p className="px-hint">{t("progress.reviews.intro")}</p>
        {overview.reviews_due.length === 0 ? (
          <p>{t("progress.reviews.empty")}</p>
        ) : (
          <ul className="rows">
            {overview.reviews_due.map((item) => (
              <ReviewItem key={`${item.item_kind}/${item.item_ref}`} item={item} titles={titles} />
            ))}
          </ul>
        )}
        {overview.reviews_due_truncated ? <p className="px-hint">{t("progress.reviews.truncated")}</p> : null}
      </div>
    </Panel>
  );
}

function UnitRow({ unit, titles }: { unit: UnitProgressView; titles: UnitTitles }) {
  const t = useT();
  return (
    <li className="rows__item">
      <div className="px-stack" style={{ "--gap": "var(--space-1)" } as React.CSSProperties}>
        <strong className="rows__text">{unitLabel(titles, unit.unit_id)}</strong>
        <span className="px-hint">
          {unit.best_checkpoint === null
            ? t("progress.units.nocheckpoint")
            : t("progress.units.checkpoint", { percent: Math.round(unit.best_checkpoint * 100) })}
        </span>
      </div>
      <Badge>{t(`progress.units.status.${unit.status}`)}</Badge>
    </li>
  );
}

/** Units the learner has a progress row for. A checkpoint result is a result on an authored task, never a level. */
export function UnitsPanel({ units, titles }: { units: readonly UnitProgressView[]; titles: UnitTitles }) {
  const t = useT();
  return (
    <Panel title={t("progress.units.title")} raised>
      {units.length === 0 ? (
        <p>{t("progress.units.empty")}</p>
      ) : (
        <ul className="rows">
          {units.map((unit) => (
            <UnitRow key={unit.unit_id} unit={unit} titles={titles} />
          ))}
        </ul>
      )}
    </Panel>
  );
}

function SessionRow({ session, titles }: { session: SessionSummary; titles: UnitTitles }) {
  const t = useT();
  const formatTime = useFormatTime();
  const role = sessionRole(session.kind);
  return (
    <li className="rows__item" data-role={role}>
      <div className="px-stack" style={{ "--gap": "var(--space-1)" } as React.CSSProperties}>
        <span className="rows__text">
          {t("progress.recent.row", {
            id: session.id,
            kind: t(`progress.kind.${session.kind}`),
            status: t(`progress.status.${session.status}`),
            time: formatTime(session.started_at),
          })}
        </span>
        {session.unit_id ? <span className="px-hint">{unitLabel(titles, session.unit_id)}</span> : null}
      </div>
      <Badge tone={role === "can_count" ? "success" : "estimate"}>{t(`progress.role.${role}`)}</Badge>
    </li>
  );
}

/**
 * Recent sessions, each marked with whether its kind can add evidence to an
 * estimate. Free modes are marked as never counting, so a chat or a generated
 * text cannot be read as part of a level.
 */
export function RecentPractice({ sessions, titles }: { sessions: readonly SessionSummary[]; titles: UnitTitles }) {
  const t = useT();
  return (
    <Panel title={t("progress.recent.title")} raised>
      {sessions.length === 0 ? (
        <p>{t("progress.recent.empty")}</p>
      ) : (
        <ul className="rows">
          {sessions.map((session) => (
            <SessionRow key={session.id} session={session} titles={titles} />
          ))}
        </ul>
      )}
    </Panel>
  );
}

/** Pronunciation: experimental, no level, and apart from the four skills. */
export function PronunciationPanel({ reviews }: { reviews: readonly ReviewDueView[] }) {
  const t = useT();
  const due = reviews.filter((item) => item.item_kind === "pron").length;
  return (
    <Panel title={t("progress.pron.title")} className="pronpanel">
      <div className="px-stack" style={{ "--gap": "var(--space-3)" } as React.CSSProperties}>
        <p className="px-row">
          <Badge tone="info">{t("progress.pron.badge")}</Badge>
          <span>{t("pron.experimental")}</span>
        </p>
        <p>{t("progress.pron.body")}</p>
        <p className="px-hint">{t("progress.pron.due", { count: due })}</p>
      </div>
    </Panel>
  );
}
