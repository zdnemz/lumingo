"use client";

import { useCallback, useState } from "react";
import { useApi } from "@/api/ApiProvider";
import { ErrorBanner } from "@/components/ErrorBanner";
import { PrivacyStatements } from "@/components/PrivacyStatements";
import { UnavailableNote } from "@/components/UnavailableNote";
import type { DiagnosticsReport } from "@/generated/DiagnosticsReport";
import type { ProgressOverview } from "@/generated/ProgressOverview";
import type { SessionSummary } from "@/generated/SessionSummary";
import { useFormatTime } from "@/providers/ProviderCard";
import { useT } from "@/state/PreferencesProvider";
import { useAction, useResource } from "@/state/useResource";
import { Button } from "@/ui/Button";
import { Panel } from "@/ui/Panel";
import { StatusBanner } from "@/ui/StatusBanner";
import { saveTextFile } from "./download";
import { TypedConfirm } from "./TypedConfirm";

/** The privacy page: what leaves the computer, where the data is, export, and the two deletes. */
export function PrivacyScreen() {
  const t = useT();
  return (
    <div className="px-stack" style={{ "--gap": "var(--space-6)" } as React.CSSProperties}>
      <Panel title={t("data.leaves.title")} raised>
        <PrivacyStatements />
      </Panel>
      <Panel title={t("data.where.title")} raised>
        <DataLocations />
      </Panel>
      <Panel title={t("data.export.title")} raised>
        <ExportSection />
      </Panel>
      <Panel title={t("data.session.title")} raised>
        <SessionDeleter />
      </Panel>
      <Panel title={t("data.all.title")} raised tone="danger">
        <DeleteEverything />
      </Panel>
      <Panel title={t("data.inspector.title")}>
        <UnavailableNote feature="inspector" />
      </Panel>
    </div>
  );
}

function DataLocations() {
  const t = useT();
  const api = useApi();
  const load = useCallback((signal: AbortSignal) => api.getDiagnostics(signal), [api]);
  const report = useResource<DiagnosticsReport>(load);
  if (report.state.status === "loading") return <p role="status">{t("state.loading")}</p>;
  if (report.state.status === "error") return <ErrorBanner error={report.state.error} onRetry={report.reload} />;
  const data = report.state.data;
  return (
    <div className="px-stack">
      <dl className="facts">
        <div>
          <dt>{t("data.where.data")}</dt>
          <dd className="phonetic facts__wrap">{data.data_dir}</dd>
        </div>
        <div>
          <dt>{t("data.where.logs")}</dt>
          <dd className={data.log_folder ? "phonetic facts__wrap" : undefined}>{data.log_folder ?? t("data.where.logs.none")}</dd>
        </div>
        <div>
          <dt>{t("data.where.units")}</dt>
          <dd className="phonetic facts__wrap">{data.curriculum_dir}</dd>
        </div>
        <div>
          <dt>{t("data.where.address")}</dt>
          <dd className="phonetic facts__wrap">{data.server_address ?? t("diag.address.unknown")}</dd>
        </div>
      </dl>
      <p className="px-hint">{t("data.where.note")}</p>
    </div>
  );
}

function ExportSection() {
  const t = useT();
  const api = useApi();
  const action = useAction();
  const [filename, setFilename] = useState<string | null>(null);

  const download = async () => {
    setFilename(null);
    const file = await action.run(() => api.exportData());
    if (file) {
      saveTextFile(file.filename, file.text);
      setFilename(file.filename);
    }
  };

  return (
    <div className="px-stack">
      <p>{t("data.export.body")}</p>
      <div>
        <Button variant="primary" busy={action.busy} onClick={() => void download()}>
          {action.busy ? t("data.export.busy") : t("data.export.button")}
        </Button>
      </div>
      {action.error ? <ErrorBanner error={action.error} onRetry={() => void download()} /> : null}
      {filename ? <p role="status">{t("data.export.done", { filename })}</p> : null}
    </div>
  );
}

interface Outcome {
  audio_files_removed: number;
  compacted: boolean;
}

/** What a delete did, including the case where the database file could not be rewritten. */
function DeleteOutcome({ message, outcome }: { message: string; outcome: Outcome }) {
  const t = useT();
  return (
    <StatusBanner tone={outcome.compacted ? "success" : "warning"} title={message}>
      <p>{t("data.deleted.audio", { count: outcome.audio_files_removed })}</p>
      {outcome.compacted ? null : <p>{t("data.deleted.notcompacted")}</p>}
    </StatusBanner>
  );
}

function SessionDeleter() {
  const t = useT();
  const api = useApi();
  const formatTime = useFormatTime();
  const load = useCallback((signal: AbortSignal) => api.getProgress(signal), [api]);
  const progress = useResource<ProgressOverview>(load);
  const action = useAction();
  const [confirming, setConfirming] = useState<SessionSummary | null>(null);
  const [outcome, setOutcome] = useState<{ id: number; result: Outcome } | null>(null);

  if (progress.state.status === "loading") return <p role="status">{t("state.loading")}</p>;
  if (progress.state.status === "error") return <ErrorBanner error={progress.state.error} onRetry={progress.reload} />;
  const sessions = progress.state.data.recent_sessions;

  const remove = async (session: SessionSummary) => {
    const result = await action.run(() => api.deleteSession(session.id));
    if (result) {
      setConfirming(null);
      setOutcome({ id: session.id, result });
      progress.refresh();
    }
  };

  return (
    <div className="px-stack">
      <p>{t("data.session.body")}</p>
      {outcome ? <DeleteOutcome message={t("data.session.done", { id: outcome.id })} outcome={outcome.result} /> : null}
      {sessions.length === 0 ? (
        <p>{t("data.session.empty")}</p>
      ) : (
        <ul className="rows">
          {sessions.map((session) => (
            <li key={session.id} className="rows__item">
              <span className="rows__text">
                {t("data.session.row", {
                  id: session.id,
                  kind: t(`data.session.kind.${session.kind}`),
                  time: formatTime(session.started_at),
                })}
                {session.unit_id ? `, ${t("data.session.unit", { unit: session.unit_id })}` : ""}
              </span>
              <Button
                small
                variant="danger"
                aria-label={t("data.session.delete", { id: session.id })}
                onClick={() => {
                  action.clear();
                  setOutcome(null);
                  setConfirming(session);
                }}
              >
                {t("common.delete")}
              </Button>
            </li>
          ))}
        </ul>
      )}
      {confirming ? (
        <TypedConfirm
          key={confirming.id}
          phrase={String(confirming.id)}
          title={t("data.session.delete", { id: confirming.id })}
          body={t("data.session.confirm.body", { id: confirming.id })}
          prompt={t("data.session.confirm.phrase", { phrase: confirming.id })}
          busy={action.busy}
          onConfirm={() => void remove(confirming)}
          onCancel={() => {
            action.clear();
            setConfirming(null);
          }}
        />
      ) : null}
      {action.error ? <ErrorBanner error={action.error} /> : null}
    </div>
  );
}

function DeleteEverything() {
  const t = useT();
  const api = useApi();
  const action = useAction();
  const [asking, setAsking] = useState(false);
  const [outcome, setOutcome] = useState<Outcome | null>(null);
  const phrase = t("data.confirm.phrase.all");

  const run = async () => {
    const result = await action.run(() => api.deleteAllData());
    if (result) {
      setAsking(false);
      setOutcome(result);
    }
  };

  return (
    <div className="px-stack">
      <p>{t("data.all.body")}</p>
      {outcome ? <DeleteOutcome message={t("data.all.done")} outcome={outcome} /> : null}
      {asking ? (
        <TypedConfirm
          phrase={phrase}
          title={t("data.all.title")}
          body={t("data.all.body")}
          prompt={t("data.all.confirm.phrase", { phrase })}
          busy={action.busy}
          onConfirm={() => void run()}
          onCancel={() => {
            action.clear();
            setAsking(false);
          }}
        />
      ) : (
        <div>
          <Button
            variant="danger"
            onClick={() => {
              action.clear();
              setOutcome(null);
              setAsking(true);
            }}
          >
            {t("data.all.button")}
          </Button>
        </div>
      )}
      {action.error ? <ErrorBanner error={action.error} /> : null}
    </div>
  );
}
