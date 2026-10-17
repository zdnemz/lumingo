"use client";

import { useServerState } from "@/api/ServerState";
import type { EngineId } from "@/generated/EngineId";
import type { EngineState } from "@/generated/EngineState";
import type { EngineView } from "@/generated/EngineView";
import type { MessageKey } from "@/i18n";
import { useT } from "@/state/PreferencesProvider";
import { Badge } from "@/ui/Badge";
import { Button } from "@/ui/Button";
import { Panel } from "@/ui/Panel";

/**
 * Every engine the program can report, named as a learner reads it. Typed as a
 * full record so a new engine on the server is a compile error here, not a
 * blank row.
 */
const ENGINE_NAME: Record<EngineId, MessageKey> = {
  audio_input: "engine.id.audio_input",
  audio_output: "engine.id.audio_output",
  vad: "engine.id.vad",
  stt: "engine.id.stt",
  tts: "engine.id.tts",
  pron: "engine.id.pron",
};

/** Where an engine stands, in words. Exhaustive for the same reason. */
const STATE_TEXT: Record<EngineState, MessageKey> = {
  unavailable: "engine.state.unavailable",
  configured: "engine.state.configured",
  loading: "engine.state.loading",
  ready: "engine.state.ready",
  failed: "engine.state.failed",
};

/** The badge colour of a state. The word next to it carries the meaning. */
function toneOf(state: EngineState): "success" | "danger" | "info" | undefined {
  if (state === "ready") return "success";
  if (state === "failed") return "danger";
  if (state === "loading") return "info";
  return undefined;
}

/**
 * What the program reported about each engine, as a list. It is pure: it takes
 * the engines and the retry, so every state can be shown from a fixture.
 *
 * The sentence under a name is the program's own (`detail`). It is rendered
 * as text and never paraphrased: this page does not know why an engine failed,
 * so it never names a cause the server did not give.
 */
export function EngineStatusPanel({ engines, onRetry }: { engines: readonly EngineView[]; onRetry: () => void }) {
  const t = useT();
  return (
    <Panel title={t("engine.panel.title")} raised>
      <div className="px-stack" style={{ "--gap": "var(--space-3)" } as React.CSSProperties}>
        <p>{t("engine.panel.intro")}</p>
        {engines.length === 0 ? (
          <p>{t("engine.panel.empty")}</p>
        ) : (
          <ul className="engine-list">
            {engines.map((engine) => (
              <li key={engine.id} className="engine-list__item">
                <p className="engine-list__head">
                  <strong>{t(ENGINE_NAME[engine.id])}</strong>
                  <Badge tone={toneOf(engine.state)}>{t(STATE_TEXT[engine.state])}</Badge>
                </p>
                <p className="engine-list__detail">{engine.detail}</p>
                {engine.model === null ? null : (
                  <p className="px-hint">
                    <span className="phonetic">{engine.model.id}</span>{" "}
                    <span>{t("settings.models.version", { version: engine.model.version })}</span>
                  </p>
                )}
              </li>
            ))}
          </ul>
        )}
        <div>
          <Button onClick={onRetry}>{t("state.retry")}</Button>
        </div>
      </div>
    </Panel>
  );
}

/**
 * The panel for the live connection: it reads the shared snapshot and hands the
 * engines to the pure panel. It lives here rather than inside
 * `DiagnosticsScreen` so that screen keeps rendering without a server state.
 */
export function EngineStatusSection() {
  const t = useT();
  const { snapshot, retry } = useServerState();
  if (snapshot === null) return <p className="px-hint">{t("state.loading")}</p>;
  if (snapshot.engines.length === 0) return <p>{t("engine.panel.empty")}</p>;
  return <EngineStatusPanel engines={snapshot.engines} onRetry={retry} />;
}
