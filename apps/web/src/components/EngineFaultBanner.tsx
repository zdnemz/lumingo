"use client";

import { useServerState } from "@/api/ServerState";
import type { ActiveSessionView } from "@/generated/ActiveSessionView";
import type { EngineFaultView } from "@/generated/EngineFaultView";
import type { MessageKey } from "@/i18n";
import { useT } from "@/state/PreferencesProvider";
import { LinkButton } from "@/ui/Button";
import { StatusBanner } from "@/ui/StatusBanner";

/**
 * The sentence that names which engine stopped. Typed as a record of every
 * server variant, so a fifth `EngineFaultView` fails the type check here
 * instead of reaching a learner as an empty banner.
 */
const FAULT_TEXT: Record<EngineFaultView, MessageKey> = {
  microphone: "engine.fault.microphone",
  speech_recognition: "engine.fault.speech_recognition",
  speech_synthesis: "engine.fault.speech_synthesis",
  playback: "engine.fault.playback",
};

/**
 * The interruption for a fault that stopped a session mid-way. It is a pure
 * function of the session it is given (the only hook is the translator), so a
 * test can render every branch from a fixture.
 *
 * It is fault-only on purpose: a banner driven by the engines' own state would
 * sit on every screen of a build without speech engines. The diagnostics panel
 * is the general surface; this is the one-off interruption.
 */
export function EngineFaultBanner({ session }: { session: ActiveSessionView | null }) {
  const t = useT();
  // The fault is only the session's live state while `life` says so. A stale
  // fault on an ended session is history, not something to interrupt for.
  if (session === null || session.life !== "engine_error" || session.fault === null) return null;
  return (
    <StatusBanner
      tone="danger"
      title={t("engine.fault.title")}
      actions={<LinkButton href="/settings/diagnostics/">{t("engine.fault.action")}</LinkButton>}
    >
      <p>{t(FAULT_TEXT[session.fault])}</p>
    </StatusBanner>
  );
}

/**
 * The shell's copy of the banner: it reads the session from the shared event
 * stream and says nothing at all before the first snapshot, when there is no
 * state to judge and a screen must not claim there is.
 */
export function EngineFaultNotice() {
  const { snapshot } = useServerState();
  return <EngineFaultBanner session={snapshot?.active_session ?? null} />;
}
