"use client";

import { useCallback, useRef, useState } from "react";
import { useApi } from "@/api/ApiProvider";
import { ErrorBanner } from "@/components/ErrorBanner";
import type { ProbeReport } from "@/generated/ProbeReport";
import type { ProviderInfo } from "@/generated/ProviderInfo";
import type { ProviderList } from "@/generated/ProviderList";
import { useT } from "@/state/PreferencesProvider";
import { useAction, useResource } from "@/state/useResource";
import { Button } from "@/ui/Button";
import { Panel } from "@/ui/Panel";
import { StatusBanner } from "@/ui/StatusBanner";
import { ProbeResult } from "./ProbeResult";
import { ProviderCard } from "./ProviderCard";
import { ProviderForm } from "./ProviderForm";

/** A copy of `record` without one key. */
function without<T>(record: Record<string, T>, key: string): Record<string, T> {
  return Object.fromEntries(Object.entries(record).filter(([name]) => name !== key));
}

type Mode = { kind: "list" } | { kind: "add" } | { kind: "edit"; name: string };

/**
 * The provider profiles of the settings screen: test, activate, edit, delete
 * with a confirmation, and add. Every failure is a banner with a way out.
 */
export function ProviderManager() {
  const t = useT();
  const api = useApi();
  const load = useCallback((signal: AbortSignal) => api.listProviders(signal), [api]);
  const list = useResource<ProviderList>(load);
  const action = useAction();
  const [mode, setMode] = useState<Mode>({ kind: "list" });
  const [reports, setReports] = useState<Record<string, ProbeReport>>({});
  const [busyId, setBusyId] = useState<number | null>(null);
  const [confirming, setConfirming] = useState<number | null>(null);
  const focusKey = useRef(false);

  const keyRef = useCallback((element: HTMLInputElement | null) => {
    if (element && focusKey.current) {
      focusKey.current = false;
      element.focus();
    }
  }, []);

  if (list.state.status === "loading") return <p role="status">{t("state.loading")}</p>;
  if (list.state.status === "error") return <ErrorBanner error={list.state.error} onRetry={list.reload} />;
  const { providers, problems } = list.state.data;

  const test = async (provider: ProviderInfo) => {
    setBusyId(provider.id);
    const report = await action.run(() => api.testProvider(provider.id));
    setBusyId(null);
    if (report) {
      setReports((previous) => ({ ...previous, [provider.name]: report }));
      list.refresh();
    }
  };

  const activate = async (provider: ProviderInfo) => {
    setBusyId(provider.id);
    const done = await action.run(() => api.activateProvider(provider.id));
    setBusyId(null);
    if (done) list.refresh();
  };

  const remove = async (provider: ProviderInfo) => {
    setBusyId(provider.id);
    const updated = await action.run(() => api.deleteProvider(provider.id));
    setBusyId(null);
    setConfirming(null);
    if (updated) {
      list.replace(updated);
      setReports((previous) => without(previous, provider.name));
    }
  };

  const edit = (provider: ProviderInfo, forKey: boolean) => {
    focusKey.current = forKey;
    action.clear();
    setMode({ kind: "edit", name: provider.name });
  };

  const saved = (savedProvider: ProviderInfo) => {
    setMode({ kind: "list" });
    // A saved profile may have a new id, so an old result no longer describes it.
    setReports((previous) => without(previous, savedProvider.name));
    list.refresh();
  };

  const editing = mode.kind === "edit" ? providers.find((p) => p.name === mode.name) : undefined;

  return (
    <div className="px-stack">
      {problems.length > 0 ? (
        <StatusBanner tone="warning" title={t("provider.problems.title")}>
          <p>{t("provider.problems.body")}</p>
          <ul className="problems">
            {problems.map((problem) => (
              <li key={problem}>{problem}</li>
            ))}
          </ul>
        </StatusBanner>
      ) : null}

      {mode.kind === "list" && action.error ? <ErrorBanner error={action.error} /> : null}

      {mode.kind === "list" ? (
        providers.length === 0 ? (
          <p>{t("provider.list.empty")}</p>
        ) : (
          <ul className="cards" aria-label={t("provider.list.label")}>
            {providers.map((provider) => {
              const report = reports[provider.name];
              const isEnv = provider.source === "env";
              const busy = busyId === provider.id && action.busy;
              return (
                <li key={provider.name}>
                  <ProviderCard
                    provider={provider}
                    actions={
                      <>
                        <Button small busy={busy} onClick={() => void test(provider)}>
                          {busy && busyId === provider.id ? t("provider.testing") : t("provider.test")}
                        </Button>
                        {provider.is_active ? null : (
                          <Button small onClick={() => void activate(provider)} disabled={action.busy}>
                            {t("provider.activate")}
                          </Button>
                        )}
                        {isEnv ? null : (
                          <>
                            <Button small onClick={() => edit(provider, false)}>
                              {t("provider.edit")}
                            </Button>
                            <Button small variant="danger" onClick={() => setConfirming(provider.id)} disabled={action.busy}>
                              {t("provider.delete")}
                            </Button>
                          </>
                        )}
                      </>
                    }
                  >
                    {report ? (
                      <ProbeResult
                        report={report}
                        busy={busy}
                        readOnly={isEnv}
                        onRetest={() => void test(provider)}
                        onEnterKey={isEnv ? undefined : () => edit(provider, true)}
                        onEdit={isEnv ? undefined : () => edit(provider, false)}
                      />
                    ) : null}
                    {confirming === provider.id ? (
                      <StatusBanner
                        tone="danger"
                        title={t("provider.delete.confirm.title", { name: provider.name })}
                        actions={
                          <>
                            <Button variant="danger" busy={action.busy} onClick={() => void remove(provider)}>
                              {t("provider.delete.confirm.yes")}
                            </Button>
                            <Button onClick={() => setConfirming(null)}>{t("provider.delete.confirm.no")}</Button>
                          </>
                        }
                      >
                        <p>{t("provider.delete.confirm.body")}</p>
                      </StatusBanner>
                    ) : null}
                  </ProviderCard>
                </li>
              );
            })}
          </ul>
        )
      ) : null}

      {mode.kind === "list" ? (
        <div>
          <Button variant="primary" onClick={() => setMode({ kind: "add" })}>
            {t("settings.providers.add")}
          </Button>
        </div>
      ) : null}

      {mode.kind === "add" ? (
        <Panel title={t("settings.providers.add")} raised>
          <ProviderForm
            makeActiveDefault={list.state.data.active_id === null}
            onSaved={saved}
            onCancel={() => setMode({ kind: "list" })}
          />
        </Panel>
      ) : null}

      {mode.kind === "edit" && editing ? (
        <Panel title={t("provider.edit")} raised>
          <ProviderForm
            initial={editing}
            makeActiveDefault={editing.is_active}
            keyRef={keyRef}
            onSaved={saved}
            onCancel={() => setMode({ kind: "list" })}
          />
        </Panel>
      ) : null}
    </div>
  );
}
