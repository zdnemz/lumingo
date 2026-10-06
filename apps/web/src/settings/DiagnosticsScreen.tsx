"use client";

import { useCallback } from "react";
import { useApi } from "@/api/ApiProvider";
import { ErrorBanner } from "@/components/ErrorBanner";
import { HardwareSummary } from "@/components/HardwareSummary";
import type { DiagnosticsReport } from "@/generated/DiagnosticsReport";
import type { Percentiles } from "@/generated/Percentiles";
import { CapabilityRows } from "@/providers/ProbeResult";
import { useT, type Translate } from "@/state/PreferencesProvider";
import { useResource } from "@/state/useResource";
import { Button } from "@/ui/Button";
import { Panel } from "@/ui/Panel";

function uptime(t: Translate, ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  if (hours > 0) return t("diag.uptime.hm", { hours, minutes });
  if (minutes > 0) return t("diag.uptime.ms", { minutes, seconds: total % 60 });
  return t("diag.uptime.s", { seconds: total });
}

function millis(t: Translate, value: number | null): string {
  return value === null ? t("diag.latency.none") : t("probe.ms", { count: Math.round(value) });
}

/** The diagnostics page (FR-O2): hardware, speed, structured-output level, folders, address. */
export function DiagnosticsScreen() {
  const t = useT();
  const api = useApi();
  const load = useCallback((signal: AbortSignal) => api.getDiagnostics(signal), [api]);
  const report = useResource<DiagnosticsReport>(load);

  return (
    <div className="px-stack" style={{ "--gap": "var(--space-6)" } as React.CSSProperties}>
      <div className="px-row">
        <p>{t("diag.intro")}</p>
        <Button onClick={report.reload}>{t("diag.refresh")}</Button>
      </div>
      {report.state.status === "loading" ? <p role="status">{t("state.loading")}</p> : null}
      {report.state.status === "error" ? <ErrorBanner error={report.state.error} onRetry={report.reload} /> : null}
      {report.state.status === "ready" ? <Report data={report.state.data} /> : null}
    </div>
  );
}

function Report({ data }: { data: DiagnosticsReport }) {
  const t = useT();
  const provider = data.provider;
  return (
    <>
      <Panel title={t("diag.program.title")} raised>
        <dl className="facts">
          <div>
            <dt>{t("diag.program.version")}</dt>
            <dd>{data.server_version}</dd>
          </div>
          <div>
            <dt>{t("diag.program.mode")}</dt>
            <dd>{data.dev_mode ? t("link.mode.dev") : t("link.mode.normal")}</dd>
          </div>
          <div>
            <dt>{t("diag.program.uptime")}</dt>
            <dd>{uptime(t, data.uptime_ms)}</dd>
          </div>
          <div>
            <dt>{t("diag.program.address")}</dt>
            <dd className="phonetic facts__wrap">{data.server_address ?? t("diag.address.unknown")}</dd>
          </div>
          <div>
            <dt>{t("diag.program.schema")}</dt>
            <dd>{data.schema_version}</dd>
          </div>
        </dl>
      </Panel>

      <Panel title={t("diag.hardware.title")} raised>
        <HardwareSummary hardware={data.hardware} />
      </Panel>

      <Panel title={t("diag.provider.title")} raised>
        {provider === null ? (
          <p>{t("diag.provider.none")}</p>
        ) : (
          <div className="px-stack">
            <dl className="facts">
              <div>
                <dt>{t("provider.name")}</dt>
                <dd>{provider.name}</dd>
              </div>
              <div>
                <dt>{t("provider.protocol")}</dt>
                <dd>{t(`provider.protocol.${provider.protocol}`)}</dd>
              </div>
              <div>
                <dt>{t("provider.model")}</dt>
                <dd className="phonetic facts__wrap">{provider.model}</dd>
              </div>
            </dl>
            {provider.capabilities ? <CapabilityRows caps={provider.capabilities} /> : <p>{t("diag.provider.untested")}</p>}
          </div>
        )}
      </Panel>

      <Panel title={t("diag.curriculum.title")} raised>
        <dl className="facts">
          <div>
            <dt>{t("diag.curriculum.units")}</dt>
            <dd>{data.curriculum.unit_count}</dd>
          </div>
          <div>
            <dt>{t("diag.curriculum.issues")}</dt>
            <dd>{data.curriculum.issue_count}</dd>
          </div>
          <div>
            <dt>{t("diag.curriculum.version")}</dt>
            <dd className="phonetic facts__wrap">{data.curriculum.content_version ?? t("diag.curriculum.none")}</dd>
          </div>
        </dl>
      </Panel>

      <Panel title={t("diag.latency.title")} raised>
        {data.latency.length === 0 ? (
          <p>{t("diag.latency.empty")}</p>
        ) : (
          <div className="table-wrap">
            <table className="table">
              <thead>
                <tr>
                  <th scope="col">{t("diag.latency.metric")}</th>
                  <th scope="col">{t("diag.latency.count")}</th>
                  <th scope="col">{t("diag.latency.p50")}</th>
                  <th scope="col">{t("diag.latency.p95")}</th>
                </tr>
              </thead>
              <tbody>
                {data.latency.map((row) => (
                  <tr key={row.metric}>
                    <th scope="row" className="phonetic">
                      {row.metric}
                    </th>
                    <td>{row.stats.count}</td>
                    <td>{millis(t, row.stats.p50_ms)}</td>
                    <td>{millis(t, row.stats.p95_ms)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Panel>

      <Panel title={t("diag.llm.title")} raised>
        {data.llm.calls === 0 ? (
          <p>{t("diag.llm.empty")}</p>
        ) : (
          <dl className="facts">
            <div>
              <dt>{t("diag.llm.calls")}</dt>
              <dd>{data.llm.calls}</dd>
            </div>
            <div>
              <dt>{t("diag.llm.failures")}</dt>
              <dd>{data.llm.failures}</dd>
            </div>
            <div>
              <dt>{t("diag.llm.ttft")}</dt>
              <dd>{pair(t, data.llm.ttft)}</dd>
            </div>
            <div>
              <dt>{t("diag.llm.total")}</dt>
              <dd>{pair(t, data.llm.total)}</dd>
            </div>
          </dl>
        )}
      </Panel>
    </>
  );
}

function pair(t: Translate, stats: Percentiles): string {
  return t("diag.llm.pair", { p50: millis(t, stats.p50_ms), p95: millis(t, stats.p95_ms) });
}
