"use client";

import type { ProbeFailureKind } from "@/generated/ProbeFailureKind";
import type { ProbeReport } from "@/generated/ProbeReport";
import type { ProviderCapabilities } from "@/generated/ProviderCapabilities";
import type { MessageKey } from "@/i18n";
import { useT, type Translate } from "@/state/PreferencesProvider";
import { Button } from "@/ui/Button";
import { StatusBanner } from "@/ui/StatusBanner";

const FAILURE_TEXT: Record<ProbeFailureKind, { title: MessageKey; body: MessageKey }> = {
  auth: { title: "probe.fail.auth.title", body: "probe.fail.auth.body" },
  network: { title: "probe.fail.network.title", body: "probe.fail.network.body" },
  timeout: { title: "probe.fail.timeout.title", body: "probe.fail.timeout.body" },
  rate_limited: { title: "probe.fail.rate_limited.title", body: "probe.fail.rate_limited.body" },
  provider_error: { title: "probe.fail.provider_error.title", body: "probe.fail.provider_error.body" },
  unexpected_reply: { title: "probe.fail.unexpected_reply.title", body: "probe.fail.unexpected_reply.body" },
  configuration: { title: "probe.fail.configuration.title", body: "probe.fail.configuration.body" },
  cancelled: { title: "probe.fail.cancelled.title", body: "probe.fail.cancelled.body" },
};

const STRUCTURED_TEXT: Record<number, MessageKey> = {
  1: "probe.structured.1",
  2: "probe.structured.2",
  3: "probe.structured.3",
  4: "probe.structured.4",
};

function yesNo(t: Translate, value: boolean): string {
  return value ? t("probe.yes") : t("probe.no");
}

function structuredText(t: Translate, level: number | null): string {
  if (level === null) return t("probe.structured.none");
  const meaning = STRUCTURED_TEXT[level];
  return meaning ? t("probe.structured.level", { level, meaning: t(meaning) }) : t("probe.structured.none");
}

function limitsText(t: Translate, caps: ProviderCapabilities): string {
  const parts: string[] = [];
  if (caps.rate_limit_rpm !== null) parts.push(t("probe.limits.rpm", { count: caps.rate_limit_rpm }));
  if (caps.rate_limit_rpd !== null) parts.push(t("probe.limits.rpd", { count: caps.rate_limit_rpd }));
  return parts.length > 0 ? parts.join(", ") : t("probe.limits.none");
}

/** What the capability test found, as plain rows (FR-A5). */
export function CapabilityRows({ caps, durationMs }: { caps: ProviderCapabilities; durationMs?: number }) {
  const t = useT();
  return (
    <dl className="facts">
      <div>
        <dt>{t("probe.row.auth")}</dt>
        <dd>{yesNo(t, caps.auth_ok)}</dd>
      </div>
      <div>
        <dt>{t("probe.row.stream")}</dt>
        <dd>{yesNo(t, caps.stream_ok)}</dd>
      </div>
      <div>
        <dt>{t("probe.row.ttft")}</dt>
        <dd>{caps.ttft_ms === null ? t("probe.unmeasured") : t("probe.ms", { count: Math.round(caps.ttft_ms) })}</dd>
      </div>
      <div>
        <dt>{t("probe.row.speed")}</dt>
        <dd>{caps.tokens_per_second === null ? t("probe.unmeasured") : t("probe.speed", { count: Math.round(caps.tokens_per_second) })}</dd>
      </div>
      <div>
        <dt>{t("probe.row.structured")}</dt>
        <dd>{structuredText(t, caps.structured_level)}</dd>
      </div>
      <div>
        <dt>{t("probe.row.contracts")}</dt>
        <dd>{caps.contracts_ok.length}</dd>
      </div>
      <div>
        <dt>{t("probe.row.limits")}</dt>
        <dd>{limitsText(t, caps)}</dd>
      </div>
      {durationMs === undefined ? null : (
        <div>
          <dt>{t("probe.row.duration")}</dt>
          <dd>{t("probe.seconds", { count: (durationMs / 1000).toFixed(1) })}</dd>
        </div>
      )}
    </dl>
  );
}

export interface ProbeResultProps {
  report: ProbeReport;
  /** Runs the test again. */
  onRetest: () => void;
  /** Takes the learner to the key field of the profile. */
  onEnterKey?: () => void;
  /** Takes the learner to the profile's form. Not offered for the read-only `env` profile. */
  onEdit?: () => void;
  busy?: boolean;
}

/**
 * The answer of `POST /api/providers/{id}/test`, said plainly. A failure says
 * what failed and offers the way out: a wrong key leads back to the key field,
 * an unreachable provider offers another try and the address.
 */
export function ProbeResult({ report, onRetest, onEnterKey, onEdit, busy }: ProbeResultProps) {
  const t = useT();
  if (report.ok && report.capabilities !== null) {
    const caps = report.capabilities;
    return (
      <div className="px-stack" style={{ "--gap": "var(--space-3)" } as React.CSSProperties}>
        <StatusBanner tone="success" title={t("probe.ok.title")}>
          <CapabilityRows caps={caps} durationMs={report.duration_ms} />
        </StatusBanner>
        {caps.stream_ok ? null : (
          <StatusBanner
            tone="warning"
            title={t("probe.nostream.title")}
            actions={onEdit ? <Button onClick={onEdit}>{t("probe.action.edit")}</Button> : undefined}
          >
            <p>{t("probe.nostream.body")}</p>
          </StatusBanner>
        )}
      </div>
    );
  }

  const kind: ProbeFailureKind = report.failure?.kind ?? "unexpected_reply";
  const text = FAILURE_TEXT[kind];
  const wrongKey = kind === "auth";
  const configuration = kind === "configuration" || kind === "unexpected_reply";
  return (
    <StatusBanner
      tone="danger"
      title={t(text.title)}
      actions={
        <>
          {wrongKey && onEnterKey ? (
            <Button variant="primary" onClick={onEnterKey}>
              {t("probe.action.edit_key")}
            </Button>
          ) : null}
          <Button busy={busy} onClick={onRetest} variant={wrongKey || configuration ? "default" : "primary"}>
            {t("probe.action.retest")}
          </Button>
          {(kind === "network" || configuration) && onEdit ? <Button onClick={onEdit}>{t("probe.action.edit")}</Button> : null}
        </>
      }
    >
      <p>{t(text.body)}</p>
      {report.failure?.message ? <p className="px-hint">{t("state.details", { text: report.failure.message })}</p> : null}
    </StatusBanner>
  );
}
