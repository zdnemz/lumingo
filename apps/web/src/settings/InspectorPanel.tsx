"use client";

import { useCallback } from "react";
import { useApi } from "@/api/ApiProvider";
import { ErrorBanner } from "@/components/ErrorBanner";
import type { InspectorReport } from "@/generated/InspectorReport";
import type { PayloadEntryView } from "@/generated/PayloadEntryView";
import { useT } from "@/state/PreferencesProvider";
import { useResource } from "@/state/useResource";

/** Shown in a slot the program has no value for, such as the elapsed time of a request that never finished. */
const NO_VALUE = "—";

/**
 * What was sent to the provider and what came back. The program removes the key
 * when it captures an entry and keeps no headers, so this is the page to check
 * that. The entries arrive oldest first, as they happened, and are drawn in
 * that order.
 */
export function InspectorPanel() {
  const t = useT();
  const api = useApi();
  const load = useCallback((signal: AbortSignal) => api.getInspector(signal), [api]);
  const inspector = useResource<InspectorReport>(load);

  if (inspector.state.status === "loading") return <p role="status">{t("state.loading")}</p>;
  if (inspector.state.status === "error") {
    return (
      <div className="px-stack">
        <p>{t("data.inspector.error")}</p>
        <ErrorBanner error={inspector.state.error} onRetry={inspector.reload} />
      </div>
    );
  }

  const report = inspector.state.data;
  return (
    <div className="px-stack">
      <p>{t("data.inspector.intro")}</p>
      <p className="px-hint">{t("data.inspector.limits", { entries: report.capacity, bytes: report.max_body_bytes })}</p>
      <p className="px-hint">{t("data.inspector.dropped", { count: report.dropped })}</p>
      {report.entries.length === 0 ? (
        <p>{t("data.inspector.empty")}</p>
      ) : (
        <ol className="inspector__list">
          {report.entries.map((entry) => (
            <Entry key={entry.id} entry={entry} />
          ))}
        </ol>
      )}
    </div>
  );
}

function Entry({ entry }: { entry: PayloadEntryView }) {
  const t = useT();
  const status = entry.status === null ? entry.outcome : `${entry.status} ${entry.outcome}`;
  const elapsed = entry.elapsed_ms === null ? NO_VALUE : `${entry.elapsed_ms} ms`;
  return (
    <li className="inspector__entry">
      <div className="px-stack" style={{ "--gap": "var(--space-2)" } as React.CSSProperties}>
        <p>{t("data.inspector.row", { id: entry.id, endpoint: entry.endpoint, status, elapsed })}</p>
        <div className="px-stack" style={{ "--gap": "var(--space-2)" } as React.CSSProperties}>
          <p className="px-hint">{t("data.inspector.request")}</p>
          <pre className="inspector__body phonetic">{entry.request_body}</pre>
          {entry.request_truncated ? <p className="px-hint">{t("data.inspector.truncated")}</p> : null}
        </div>
        {entry.response_body === null ? null : (
          <div className="px-stack" style={{ "--gap": "var(--space-2)" } as React.CSSProperties}>
            <p className="px-hint">{t("data.inspector.response")}</p>
            <pre className="inspector__body phonetic">{entry.response_body}</pre>
            {entry.response_truncated ? <p className="px-hint">{t("data.inspector.truncated")}</p> : null}
          </div>
        )}
      </div>
    </li>
  );
}
