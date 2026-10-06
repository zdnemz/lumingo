"use client";

import { useCallback } from "react";
import { useApi } from "@/api/ApiProvider";
import { ApiError } from "@/api/client";
import { ErrorBanner } from "@/components/ErrorBanner";
import type { AttemptEvidence } from "@/generated/AttemptEvidence";
import type { EvidenceKind } from "@/generated/EvidenceKind";
import type { EvidenceView } from "@/generated/EvidenceView";
import { useFormatTime } from "@/providers/ProviderCard";
import { useT, type Translate } from "@/state/PreferencesProvider";
import { useResource } from "@/state/useResource";
import { Badge } from "@/ui/Badge";
import { Panel } from "@/ui/Panel";
import { StatusBanner } from "@/ui/StatusBanner";
import { parseProvenance, type Provenance } from "./model";

const KIND_ORDER: readonly EvidenceKind[] = ["response_text", "quote", "scorer_reason", "metric"];

/** The scorer types the program writes. Anything else is shown as the program sent it. */
function scorerName(t: Translate, scorer: string): string {
  switch (scorer) {
    case "deterministic":
      return t("progress.scorer.deterministic");
    case "rubric_llm":
      return t("progress.scorer.rubric_llm");
    case "pron_engine":
      return t("progress.scorer.pron_engine");
    default:
      return scorer;
  }
}

function isReferenceText(row: EvidenceView): boolean {
  const data = row.data;
  return typeof data === "object" && data !== null && !Array.isArray(data) && data.kind === "reference_text";
}

/** The scorer and engine versions behind one row, as the recorder stored them. */
function ProvenanceFacts({ provenance }: { provenance: Provenance | null }) {
  const t = useT();
  if (provenance === null) return <p className="px-hint">{t("progress.evidence.noversion")}</p>;
  return (
    <div className="px-stack" style={{ "--gap": "var(--space-2)" } as React.CSSProperties}>
      <dl className="facts">
        {provenance.scorer ? (
          <div>
            <dt>{t("progress.evidence.scorer")}</dt>
            <dd>{scorerName(t, provenance.scorer)}</dd>
          </div>
        ) : null}
        {provenance.scorerVersion ? (
          <div>
            <dt>{t("progress.evidence.scorer_version")}</dt>
            <dd className="phonetic facts__wrap">{provenance.scorerVersion}</dd>
          </div>
        ) : null}
        {provenance.normVersion ? (
          <div>
            <dt>{t("progress.evidence.norm_version")}</dt>
            <dd className="phonetic facts__wrap">{provenance.normVersion}</dd>
          </div>
        ) : null}
        {provenance.algorithmVersion ? (
          <div>
            <dt>{t("progress.evidence.algorithm")}</dt>
            <dd className="phonetic facts__wrap">{provenance.algorithmVersion}</dd>
          </div>
        ) : null}
        {provenance.recorder ? (
          <div>
            <dt>{t("progress.evidence.recorder")}</dt>
            <dd className="phonetic facts__wrap">{provenance.recorder}</dd>
          </div>
        ) : null}
      </dl>
      {provenance.engines.length > 0 ? (
        <div>
          <p className="px-label">{t("progress.evidence.engines")}</p>
          <ul className="evidence__list">
            {provenance.engines.map((engine) => (
              <li key={`${engine.role}/${engine.id}`} className="phonetic">
                {t("progress.evidence.engine", { role: engine.role, id: engine.id, version: engine.version })}
              </li>
            ))}
          </ul>
        </div>
      ) : null}
      {provenance.details.length > 0 ? (
        <div>
          <p className="px-label">{t("progress.evidence.details")}</p>
          <dl className="facts">
            {provenance.details.map(([key, value]) => (
              <div key={key}>
                <dt className="phonetic">{key}</dt>
                <dd className="phonetic facts__wrap">{value}</dd>
              </div>
            ))}
          </dl>
        </div>
      ) : null}
      {provenance.scorer === "pron_engine" ? <Badge tone="info">{t("pron.experimental")}</Badge> : null}
    </div>
  );
}

function Row({ kind, row }: { kind: EvidenceKind; row: EvidenceView }) {
  const t = useT();
  const formatTime = useFormatTime();
  return (
    <li className="evidence__row">
      {kind === "metric" ? (
        <ProvenanceFacts provenance={parseProvenance(row.data)} />
      ) : kind === "scorer_reason" ? (
        <p lang="en">{row.content}</p>
      ) : (
        // Learner text and quotes are text nodes, in the learner's own words.
        <blockquote className="evidence__text" lang="en">
          {isReferenceText(row) ? <span className="px-hint">{t("progress.evidence.reference")}</span> : null}
          {row.content}
        </blockquote>
      )}
      <p className="px-hint">{t("progress.evidence.stored", { time: formatTime(row.created_at) })}</p>
    </li>
  );
}

function Evidence({ data }: { data: AttemptEvidence }) {
  const t = useT();
  const groups = KIND_ORDER.map((kind) => ({ kind, rows: data.evidence.filter((row) => row.kind === kind) })).filter((group) => group.rows.length > 0);
  return (
    <div className="px-stack">
      <p className="px-hint">{t("progress.evidence.note")}</p>
      <dl className="facts">
        <div>
          <dt>{t("progress.evidence.activity")}</dt>
          <dd className="phonetic facts__wrap">{data.activity_id}</dd>
        </div>
        <div>
          <dt>{t("progress.evidence.skill")}</dt>
          <dd className="phonetic facts__wrap">{data.skill}</dd>
        </div>
        <div>
          <dt>{t("progress.evidence.dimension")}</dt>
          <dd className="phonetic facts__wrap">{data.dimension}</dd>
        </div>
      </dl>
      {groups.length === 0 ? <p>{t("progress.evidence.empty")}</p> : null}
      {groups.map((group) => (
        <section key={group.kind} className="px-stack" style={{ "--gap": "var(--space-2)" } as React.CSSProperties} aria-label={t(`progress.evidence.kind.${group.kind}`)}>
          <h4>{t(`progress.evidence.kind.${group.kind}`)}</h4>
          <ul className="evidence__list">
            {group.rows.map((row) => (
              <Row key={row.id} kind={group.kind} row={row} />
            ))}
          </ul>
        </section>
      ))}
    </div>
  );
}

/**
 * The stored evidence of one attempt from `GET /api/attempts/{id}/evidence`:
 * the answer, quotes, the scorer's reasons and the versions of the scorer and
 * engines. It never shows a level, a score or a percentage, because one
 * attempt does not say what level a learner is at, and this answer does not
 * say whether the attempt was authored, generated or from a free mode.
 * Mount it with `key={attemptId}` so a new number starts from loading.
 */
export function AttemptEvidencePanel({ attemptId }: { attemptId: number }) {
  const t = useT();
  const api = useApi();
  const load = useCallback((signal: AbortSignal) => api.getAttemptEvidence(attemptId, signal), [api, attemptId]);
  const evidence = useResource<AttemptEvidence>(load);
  const state = evidence.state;
  return (
    <Panel as="div" inset title={t("progress.evidence.title", { id: attemptId })} aria-live="polite">
      {state.status === "loading" ? <p role="status">{t("state.loading")}</p> : null}
      {state.status === "error" ? (
        state.error instanceof ApiError && state.error.code === "not_found" ? (
          <StatusBanner tone="info" title={t("progress.evidence.notfound")} />
        ) : (
          <ErrorBanner error={state.error} onRetry={evidence.reload} />
        )
      ) : null}
      {state.status === "ready" ? <Evidence data={state.data} /> : null}
    </Panel>
  );
}
