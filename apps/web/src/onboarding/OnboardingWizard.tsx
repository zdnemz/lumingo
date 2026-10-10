"use client";

import { useRouter } from "next/navigation";
import { useCallback, useEffect, useRef, useState } from "react";
import { useApi } from "@/api/ApiProvider";
import { useServerState } from "@/api/ServerState";
import { ErrorBanner } from "@/components/ErrorBanner";
import { HardwareSummary } from "@/components/HardwareSummary";
import { PrivacyStatements } from "@/components/PrivacyStatements";
import { UnavailableNote, useUnavailable } from "@/components/UnavailableNote";
import type { ProbeReport } from "@/generated/ProbeReport";
import type { ProviderInfo } from "@/generated/ProviderInfo";
import type { ProviderList } from "@/generated/ProviderList";
import type { UnitList } from "@/generated/UnitList";
import type { MessageKey } from "@/i18n";
import { CapabilityRows, ProbeResult } from "@/providers/ProbeResult";
import { KeyGuidePanel } from "@/providers/KeyGuidePanel";
import { ProviderCard } from "@/providers/ProviderCard";
import { ProviderForm } from "@/providers/ProviderForm";
import type { ProviderPreset } from "@/providers/keyGuide";
import { useLanguage } from "@/settings/useLanguage";
import { Mascot } from "@/sprites/Mascot";
import { useT } from "@/state/PreferencesProvider";
import { LANGUAGES } from "@/state/preferences";
import { useAction, useResource } from "@/state/useResource";
import { Button } from "@/ui/Button";
import { DialogBox } from "@/ui/DialogBox";
import { Panel } from "@/ui/Panel";
import { Segmented } from "@/ui/Segmented";
import { StatusBanner } from "@/ui/StatusBanner";
import { STEPS, loadProgress, saveProgress, startStep, stepIndex, type StepId } from "./progress";

const STEP_NAME: Record<StepId, MessageKey> = {
  welcome: "onb.step.welcome",
  language: "onb.step.language",
  privacy: "onb.step.privacy",
  guide: "onb.step.guide",
  provider: "onb.step.provider",
  test: "onb.step.test",
  hardware: "onb.step.hardware",
  ready: "onb.step.ready",
};

/** Loads the profiles first, because where the wizard may stand depends on whether one exists. */
export function OnboardingWizard() {
  const t = useT();
  const api = useApi();
  const load = useCallback((signal: AbortSignal) => api.listProviders(signal), [api]);
  const list = useResource<ProviderList>(load);
  if (list.state.status === "loading") return <p role="status">{t("state.loading")}</p>;
  if (list.state.status === "error") return <ErrorBanner error={list.state.error} onRetry={list.reload} />;
  return <Wizard providers={list.state.data} refresh={list.refresh} />;
}

interface WizardProps {
  providers: ProviderList;
  refresh: () => void;
}

function Wizard({ providers, refresh }: WizardProps) {
  const t = useT();
  const router = useRouter();
  const [stored] = useState(() => loadProgress(window));
  const [step, setStep] = useState<StepId>(() => startStep(stored, { hasProvider: providers.providers.length > 0 }));
  const [done, setDone] = useState(stored?.done ?? false);
  // Set by the key guide, read by the provider form. Never holds a key.
  const [preset, setPreset] = useState<ProviderPreset | undefined>(undefined);
  // The saved profile whose form is open on the provider step, for "enter the key again".
  const [editing, setEditing] = useState<string | null>(null);
  const focusKey = useRef(false);
  // The wrong-key recovery puts focus in the key field, so the heading must not take it back.
  const keepFocus = useRef(false);
  const heading = useRef<HTMLHeadingElement>(null);
  const firstRender = useRef(true);

  useEffect(() => {
    saveProgress(window, { step, done });
  }, [step, done]);

  // Moving on moves focus to the new step's heading, so a keyboard or screen reader user lands in the right place.
  useEffect(() => {
    if (firstRender.current) {
      firstRender.current = false;
      return;
    }
    if (keepFocus.current) {
      keepFocus.current = false;
      return;
    }
    heading.current?.focus();
  }, [step]);

  const index = stepIndex(step);
  const previous = STEPS[index - 1];
  const next = STEPS[index + 1];
  const hasProvider = providers.providers.length > 0;
  const active = providers.providers.find((p) => p.is_active) ?? providers.providers[0];

  const keyRef = useCallback((element: HTMLInputElement | null) => {
    if (element && focusKey.current) {
      focusKey.current = false;
      element.focus();
    }
  }, []);

  const enterKeyAgain = (provider: ProviderInfo) => {
    focusKey.current = true;
    keepFocus.current = true;
    setEditing(provider.name);
    setStep("provider");
  };

  const finish = () => {
    setDone(true);
    saveProgress(window, { step: "ready", done: true });
    router.push("/");
  };

  // The next button of a step may be held back until the step's job is done.
  const canContinue = step === "provider" ? hasProvider : step === "test" ? active !== undefined && active.probed_at !== null : true;

  return (
    <div className="px-stack" style={{ "--gap": "var(--space-5)" } as React.CSSProperties}>
      <nav aria-label={t("onb.steps")}>
        <ol className="wizard__steps">
          {STEPS.map((id, position) => (
            <li
              key={id}
              className="wizard__step"
              aria-current={id === step ? "step" : undefined}
              data-done={position < index ? "true" : undefined}
            >
              {t(STEP_NAME[id])}
            </li>
          ))}
        </ol>
      </nav>
      <p className="px-hint" role="status">
        {t("onb.progress", { current: index + 1, total: STEPS.length, name: t(STEP_NAME[step]) })}
      </p>

      <Panel raised aria-labelledby="wizard-title">
        <div className="px-stack">
          <h1 id="wizard-title" ref={heading} tabIndex={-1} className="wizard__title">
            {t(TITLE[step])}
          </h1>
          {step === "welcome" ? <WelcomeStep /> : null}
          {step === "language" ? <LanguageStep /> : null}
          {step === "privacy" ? <PrivacyStep /> : null}
          {step === "guide" ? <GuideStep filled={preset !== undefined} onFill={setPreset} /> : null}
          {step === "provider" ? (
            <ProviderStep
              providers={providers}
              preset={preset}
              editing={editing}
              keyRef={keyRef}
              onEdit={(name) => setEditing(name)}
              onSaved={() => {
                setEditing(null);
                setPreset(undefined);
                refresh();
              }}
              onGuide={() => setStep("guide")}
            />
          ) : null}
          {step === "test" ? <TestStep provider={active} refresh={refresh} onEnterKey={enterKeyAgain} onSkip={() => next && setStep(next)} /> : null}
          {step === "hardware" ? <HardwareStep /> : null}
          {step === "ready" ? <ReadyStep provider={active} /> : null}
        </div>
      </Panel>

      <div className="wizard__nav">
        {previous ? <Button onClick={() => setStep(previous)}>{t("onb.back")}</Button> : <span />}
        {next ? (
          <Button variant="primary" disabled={!canContinue} onClick={() => setStep(next)}>
            {t("onb.next")}
          </Button>
        ) : (
          <Button variant="primary" onClick={finish}>
            {t("onb.finish")}
          </Button>
        )}
      </div>
    </div>
  );
}

const TITLE: Record<StepId, MessageKey> = {
  welcome: "onb.welcome.title",
  language: "onb.language.title",
  privacy: "onb.privacy.title",
  guide: "onb.guide.title",
  provider: "onb.provider.title",
  test: "onb.test.title",
  hardware: "onb.hardware.title",
  ready: "onb.ready.title",
};

function WelcomeStep() {
  const t = useT();
  return (
    <>
      <div className="hero">
        <Mascot mood="happy" scale={8} />
        <div className="px-stack">
          <DialogBox name={t("chat.tutor")}>{t("onb.welcome.lumi")}</DialogBox>
        </div>
      </div>
      <p>{t("onb.welcome.body")}</p>
      <p>{t("onb.welcome.plan")}</p>
    </>
  );
}

function LanguageStep() {
  const t = useT();
  const { language, setLanguage, error } = useLanguage();
  return (
    <>
      <p>{t("onb.language.body")}</p>
      <Segmented
        legend={t("settings.language")}
        value={language}
        onChange={(value) => void setLanguage(value)}
        options={LANGUAGES.map((value) => ({ value, label: t(`settings.language.${value}`) }))}
      />
      {error ? (
        <>
          <p className="px-error">{t("onb.language.error")}</p>
          <ErrorBanner error={error} />
        </>
      ) : null}
    </>
  );
}

function PrivacyStep() {
  const t = useT();
  return (
    <>
      <p>{t("onb.privacy.body")}</p>
      <PrivacyStatements />
    </>
  );
}

function GuideStep({ filled, onFill }: { filled: boolean; onFill: (preset: ProviderPreset) => void }) {
  const t = useT();
  return (
    <>
      <p>{t("onb.guide.body")}</p>
      <KeyGuidePanel onFill={onFill} filled={filled} />
    </>
  );
}

interface ProviderStepProps {
  providers: ProviderList;
  preset: ProviderPreset | undefined;
  editing: string | null;
  keyRef: (element: HTMLInputElement | null) => void;
  onEdit: (name: string | null) => void;
  onSaved: () => void;
  onGuide: () => void;
}

function ProviderStep({ providers, preset, editing, keyRef, onEdit, onSaved, onGuide }: ProviderStepProps) {
  const t = useT();
  const [adding, setAdding] = useState(false);
  const [savedName, setSavedName] = useState<string | null>(null);
  const hasEnv = providers.providers.some((p) => p.source === "env");
  const editingProvider = editing === null ? undefined : providers.providers.find((p) => p.name === editing);
  const showAddForm = editingProvider === undefined && (providers.providers.length === 0 || adding);

  return (
    <>
      <p>{t("onb.provider.body")}</p>
      {hasEnv ? (
        <StatusBanner tone="info" title={t("onb.provider.env.title")}>
          <p>{t("onb.provider.env.body")}</p>
        </StatusBanner>
      ) : null}
      {savedName ? <p role="status">{t("onb.provider.saved", { name: savedName })}</p> : null}

      {providers.providers.length > 0 ? (
        <ul className="cards" aria-label={t("provider.list.label")}>
          {providers.providers.map((provider) => (
            <li key={provider.name}>
              <ProviderCard
                provider={provider}
                actions={
                  provider.source === "file" ? (
                    <Button small onClick={() => onEdit(provider.name)}>
                      {t("provider.edit")}
                    </Button>
                  ) : undefined
                }
              />
            </li>
          ))}
        </ul>
      ) : (
        <p className="px-hint">{t("onb.provider.required")}</p>
      )}

      {editingProvider ? (
        <Panel title={t("provider.edit")} inset>
          <ProviderForm
            initial={editingProvider}
            makeActiveDefault={editingProvider.is_active}
            keyRef={keyRef}
            onSaved={(saved) => {
              setSavedName(saved.name);
              onSaved();
            }}
            onCancel={() => onEdit(null)}
          />
        </Panel>
      ) : null}

      {showAddForm ? (
        <Panel title={t("settings.providers.add")} inset>
          <ProviderForm
            preset={preset}
            makeActiveDefault
            onSaved={(saved) => {
              setSavedName(saved.name);
              setAdding(false);
              onSaved();
            }}
            onCancel={providers.providers.length > 0 ? () => setAdding(false) : undefined}
          />
        </Panel>
      ) : null}

      <div className="px-row">
        {!showAddForm && editingProvider === undefined ? <Button onClick={() => setAdding(true)}>{t("onb.provider.another")}</Button> : null}
        <Button variant="ghost" onClick={onGuide}>
          {t("onb.provider.guide")}
        </Button>
      </div>
    </>
  );
}

interface TestStepProps {
  provider: ProviderInfo | undefined;
  refresh: () => void;
  onEnterKey: (provider: ProviderInfo) => void;
  onSkip: () => void;
}

function TestStep({ provider, refresh, onEnterKey, onSkip }: TestStepProps) {
  const t = useT();
  const api = useApi();
  const action = useAction();
  const [report, setReport] = useState<ProbeReport | null>(null);

  if (provider === undefined) {
    return <StatusBanner tone="warning" title={t("onb.test.noprovider")} />;
  }
  const isEnv = provider.source === "env";

  const run = async () => {
    const result = await action.run(() => api.testProvider(provider.id));
    if (result) {
      setReport(result);
      refresh();
    }
  };

  // A test that passed earlier is kept by the program, so a reloaded tab still knows it.
  const earlier = report === null && provider.probed_at !== null && provider.capabilities !== null;

  return (
    <>
      <p>{t("onb.test.body")}</p>
      <p>{t("onb.test.for", { name: provider.name })}</p>
      <div className="px-row">
        {report === null || report.ok ? (
          <Button variant="primary" busy={action.busy} onClick={() => void run()}>
            {action.busy ? t("onb.test.running") : report || earlier ? t("onb.test.again") : t("onb.test.run")}
          </Button>
        ) : null}
        <Button variant="ghost" onClick={onSkip}>
          {t("onb.test.skip")}
        </Button>
      </div>
      {action.error ? <ErrorBanner error={action.error} onRetry={() => void run()} /> : null}
      {report ? (
        <ProbeResult
          report={report}
          busy={action.busy}
          readOnly={isEnv}
          onRetest={() => void run()}
          onEnterKey={isEnv ? undefined : () => onEnterKey(provider)}
          onEdit={isEnv ? undefined : () => onEnterKey(provider)}
        />
      ) : null}
      {earlier && provider.capabilities ? (
        <StatusBanner tone="success" title={t("onb.test.passed")}>
          <CapabilityRows caps={provider.capabilities} />
        </StatusBanner>
      ) : null}
    </>
  );
}

function HardwareStep() {
  const t = useT();
  const { snapshot } = useServerState();
  return (
    <>
      <p>{t("onb.hardware.body")}</p>
      {snapshot ? <HardwareSummary hardware={snapshot.hardware} /> : <p role="status">{t("state.loading")}</p>}
      <section className="px-stack" aria-labelledby="onb-mic" style={{ "--gap": "var(--space-2)" } as React.CSSProperties}>
        <h2 id="onb-mic">{t("onb.hardware.mic")}</h2>
        <UnavailableNote feature="speech" />
      </section>
      <section className="px-stack" aria-labelledby="onb-models" style={{ "--gap": "var(--space-2)" } as React.CSSProperties}>
        <h2 id="onb-models">{t("onb.hardware.models")}</h2>
        <UnavailableNote feature="models" />
      </section>
      <section className="px-stack" aria-labelledby="onb-level" style={{ "--gap": "var(--space-2)" } as React.CSSProperties}>
        <h2 id="onb-level">{t("onb.hardware.level")}</h2>
        <p>{t("onb.hardware.level.body")}</p>
        <UnavailableNote feature="sessions" />
      </section>
    </>
  );
}

function ReadyStep({ provider }: { provider: ProviderInfo | undefined }) {
  const t = useT();
  const api = useApi();
  const { snapshot } = useServerState();
  const unavailable = useUnavailable();
  const { language } = useLanguage();
  const load = useCallback((signal: AbortSignal) => api.listUnits(signal), [api]);
  const units = useResource<UnitList>(load);

  const providerLine =
    provider === undefined
      ? t("onb.ready.provider.none")
      : provider.probed_at !== null
        ? t("onb.ready.provider.tested", { name: provider.name })
        : t("onb.ready.provider.untested", { name: provider.name });

  return (
    <>
      <p>{t("onb.ready.body")}</p>
      <dl className="facts">
        <div>
          <dt>{t("onb.ready.language")}</dt>
          <dd>{t(`settings.language.${language}`)}</dd>
        </div>
        <div>
          <dt>{t("onb.ready.provider")}</dt>
          <dd>{providerLine}</dd>
        </div>
        <div>
          <dt>{t("onb.ready.units")}</dt>
          <dd>
            {units.state.status === "loading" ? t("state.loading") : null}
            {units.state.status === "ready"
              ? units.state.data.units.length > 0
                ? t("onb.ready.units.count", { count: units.state.data.units.length })
                : t("onb.ready.units.none")
              : null}
            {units.state.status === "ready" && units.state.data.issues.length > 0
              ? ` ${t("onb.ready.units.issues", { count: units.state.data.issues.length })}`
              : null}
          </dd>
        </div>
      </dl>
      {units.state.status === "error" ? <ErrorBanner error={units.state.error} onRetry={units.reload} /> : null}
      {snapshot ? (
        <div>
          <h2>{t("onb.hardware.title")}</h2>
          <HardwareSummary hardware={snapshot.hardware} />
        </div>
      ) : null}
      <section className="px-stack" aria-labelledby="onb-missing" style={{ "--gap": "var(--space-2)" } as React.CSSProperties}>
        <h2 id="onb-missing">{t("onb.ready.missing")}</h2>
        {unavailable === null ? null : unavailable.length === 0 ? (
          <p>{t("onb.ready.missing.none")}</p>
        ) : (
          <>
            <p className="px-hint">{t("onb.ready.missing.body")}</p>
            <ul className="statements">
              {unavailable.map((feature) => (
                <li key={feature}>{t(`feature.${feature}`)}</li>
              ))}
            </ul>
          </>
        )}
      </section>
    </>
  );
}
