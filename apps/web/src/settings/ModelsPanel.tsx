"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { useApi } from "@/api/ApiProvider";
import { useServerState } from "@/api/ServerState";
import { ErrorBanner } from "@/components/ErrorBanner";
import type { DownloadRequest } from "@/generated/DownloadRequest";
import type { ModelList } from "@/generated/ModelList";
import type { ModelView } from "@/generated/ModelView";
import type { ServerEvent } from "@/generated/ServerEvent";
import { usePreferences, useT, type Translate } from "@/state/PreferencesProvider";
import { useAction, useResource } from "@/state/useResource";
import { Badge } from "@/ui/Badge";
import { Button } from "@/ui/Button";
import { Panel } from "@/ui/Panel";

/** One `DownloadProgress` event, the only stream event this panel reads. */
type ProgressEvent = Extract<ServerEvent, { type: "DownloadProgress" }>;

/** Shown where the server declared no total size for a file. */
const NO_VALUE = "—";

/** The newest progress of one model. The stream keeps its events newest first. */
function newestProgress(events: readonly ServerEvent[], modelId: string): ProgressEvent | null {
  for (const event of events) {
    if (event.type === "DownloadProgress" && event.model_id === modelId) return event;
  }
  return null;
}

/**
 * A byte count in the units a download is usually written in, localised. Zero
 * means the manifest carries no measured size yet, which is said as "None"
 * rather than as a size of nothing.
 */
function sizeText(t: Translate, language: string, bytes: number): string {
  if (bytes <= 0) return t("settings.models.none");
  const unit = bytes >= 1_000_000_000 ? "gigabyte" : bytes >= 1_000_000 ? "megabyte" : bytes >= 1_000 ? "kilobyte" : "byte";
  const scale = bytes >= 1_000_000_000 ? 1_000_000_000 : bytes >= 1_000_000 ? 1_000_000 : bytes >= 1_000 ? 1_000 : 1;
  return new Intl.NumberFormat(language, { style: "unit", unit, unitDisplay: "long", maximumFractionDigits: 1 }).format(bytes / scale);
}

/**
 * The speech models of the manifest the program ships. Every row is one entry
 * as the server sends it: what it is, what it is licensed under, whether it is
 * installed, and then either the server's own reason it cannot be downloaded
 * or the licence to accept and the control to fetch it. Progress arrives as
 * `DownloadProgress` events; the list is read again when a download ends, so
 * what a row says is installed is what the program really stored.
 */
export function ModelsPanel() {
  const t = useT();
  const api = useApi();
  const load = useCallback((signal: AbortSignal) => api.listModels(signal), [api]);
  const models = useResource<ModelList>(load);

  return (
    <Panel title={t("settings.models.title")}>
      {models.state.status === "loading" ? (
        <p role="status">{t("state.loading")}</p>
      ) : models.state.status === "error" ? (
        <div className="px-stack">
          <p>{t("settings.models.error")}</p>
          <ErrorBanner error={models.state.error} onRetry={models.reload} />
        </div>
      ) : (
        <ModelList models={models.state.data.models} onChanged={models.refresh} />
      )}
    </Panel>
  );
}

function ModelList({ models, onChanged }: { models: readonly ModelView[]; onChanged: () => void }) {
  const t = useT();
  return (
    <div className="px-stack">
      <p>{t("settings.models.intro")}</p>
      {models.length === 0 ? (
        <p>{t("settings.models.empty")}</p>
      ) : (
        <ul className="models">
          {models.map((model) => (
            <ModelRow key={model.id} model={model} onChanged={onChanged} />
          ))}
        </ul>
      )}
    </div>
  );
}

function ModelRow({ model, onChanged }: { model: ModelView; onChanged: () => void }) {
  const t = useT();
  const api = useApi();
  const { display } = usePreferences();
  const { events } = useServerState();
  const action = useAction();
  const [accepted, setAccepted] = useState(false);
  const [started, setStarted] = useState(false);
  // The newest sequence number of this model when the control was pressed. An
  // end event at or below it belongs to an earlier download: the server's
  // sequence number only grows, so a new end event is always above it.
  const [startedAt, setStartedAt] = useState(0);

  const progress = newestProgress(events, model.id);
  const ended = progress !== null && progress.state !== "running" && progress.seq > startedAt;
  // The server's own flag covers a download that was already running when the
  // page was opened; the event covers the window before the list is read again.
  const running = !ended && (started || model.downloading || (progress !== null && progress.state === "running"));
  const failure = ended && progress !== null && progress.state === "failed" ? progress.message : null;

  // The last event of a download changes what is installed and clears the
  // server's `downloading` flag, so the list is read again once per end.
  const seen = useRef<number | null>(null);
  useEffect(() => {
    if (!ended || progress === null || seen.current === progress.seq) return;
    seen.current = progress.seq;
    setStarted(false);
    onChanged();
  }, [ended, progress, onChanged]);

  const download = async () => {
    const request: DownloadRequest = {
      accept_licence: true,
      license: model.licence.license,
      license_url: model.licence.license_url,
    };
    setStartedAt(progress?.seq ?? 0);
    const answer = await action.run(() => api.downloadModel(model.id, request));
    if (answer) {
      setAccepted(false);
      setStarted(true);
      onChanged();
    }
  };

  const cancel = async () => {
    if (await action.run(() => api.cancelModel(model.id))) onChanged();
  };

  return (
    <li className="models__item">
      <div className="px-row">
        <h3>{model.id}</h3>
        <Badge tone="info">{t(`settings.models.role.${model.role}`)}</Badge>
      </div>
      <div className="px-row">
        <span className="phonetic">{model.engine}</span>
        <span>{t("settings.models.version", { version: model.version })}</span>
        <span>{t("settings.models.size", { size: sizeText(t, display.language, model.size_bytes) })}</span>
      </div>
      <p className="px-row">
        <span>{t("settings.models.licence", { license: model.licence.license })}</span>
        {model.licence.license_url === "" ? null : (
          <a href={model.licence.license_url} target="_blank" rel="noopener noreferrer">
            {t("settings.models.licence.open")}
          </a>
        )}
      </p>
      <p className="px-hint">
        {model.installed === null
          ? t("settings.models.notinstalled")
          : t("settings.models.installed", {
              version: model.installed.version,
              files: model.installed.files,
              size: sizeText(t, display.language, model.installed.size_bytes),
            })}
      </p>
      {model.downloadable.state === "no" ? (
        // The program says why this entry cannot be fetched. No control here
        // pretends that it could be.
        <p className="px-hint">{t("settings.models.why", { reason: model.downloadable.reason })}</p>
      ) : running ? (
        <div className="px-stack" style={{ "--gap": "var(--space-2)" } as React.CSSProperties}>
          {progress === null ? null : (
            <p role="status">
              {t("settings.models.progress", {
                file: progress.file,
                index: progress.file_index + 1,
                files: progress.file_count,
                done: progress.bytes_done,
                total: progress.bytes_total ?? NO_VALUE,
              })}
            </p>
          )}
          <div className="px-row">
            <Button small onClick={() => void cancel()}>
              {t("settings.models.cancel")}
            </Button>
          </div>
        </div>
      ) : (
        <div className="px-stack" style={{ "--gap": "var(--space-2)" } as React.CSSProperties}>
          {failure === null ? null : (
            // The sentence is the program's own: it names the kind of failure
            // and never a path or an address.
            <p role="status">{failure}</p>
          )}
          <label className="px-row">
            <input type="checkbox" checked={accepted} onChange={(event) => setAccepted(event.target.checked)} />
            <span>{t("settings.models.accept")}</span>
          </label>
          <div className="px-row">
            <Button variant="primary" busy={action.busy} disabled={!accepted} onClick={() => void download()}>
              {action.busy ? t("settings.models.download.busy") : t("settings.models.download")}
            </Button>
          </div>
        </div>
      )}
      {action.error ? <ErrorBanner error={action.error} /> : null}
    </li>
  );
}
