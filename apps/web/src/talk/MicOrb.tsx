"use client";

import { Mascot } from "@/sprites/Mascot";
import type { LumiMood } from "@/sprites/data";
import { useT } from "@/state/PreferencesProvider";
import { Button } from "@/ui/Button";
import { Meter } from "@/ui/Meter";

export type MicState = "idle" | "listening" | "transcribing" | "thinking" | "speaking" | "paused" | "error";

export interface MicOrbProps {
  state: MicState;
  /** Microphone level, 0 to 100. It is a number from the server, never sound. */
  level: number;
  /** The learner pressed stop while Lumi was speaking. */
  onStopSpeaking?: () => void;
  onPause?: () => void;
  onResume?: () => void;
}

const MOODS: Record<MicState, LumiMood> = {
  idle: "idle",
  listening: "happy",
  transcribing: "think",
  thinking: "think",
  speaking: "talk",
  paused: "idle",
  error: "sad",
};

/**
 * Lumi's face for the voice screen. The microphone belongs to the server, so
 * nothing here records: this shows what the server reports and sends the
 * learner's stop, pause and resume.
 */
export function MicOrb({ state, level, onStopSpeaking, onPause, onResume }: MicOrbProps) {
  const t = useT();
  const label = t(`mic.state.${state}`);
  return (
    <section className="orb" aria-label={t("mic.orb")}>
      <div className="orb__face" data-state={state} data-anim={state === "listening" ? "glow" : undefined}>
        <Mascot mood={MOODS[state]} scale={8} />
      </div>
      <p className="orb__state" role="status" aria-live="polite">
        {label}
      </p>
      <div className="orb__meter">
        <Meter value={state === "listening" ? level : 0} label={t("mic.level")} tone="speaking" live={state === "listening"} />
      </div>
      <div className="px-row">
        {state === "speaking" && onStopSpeaking ? (
          <Button variant="danger" onClick={onStopSpeaking}>
            {t("mic.stop")}
          </Button>
        ) : null}
        {state === "paused" && onResume ? (
          <Button variant="primary" onClick={onResume}>
            {t("mic.resume")}
          </Button>
        ) : null}
        {state !== "paused" && state !== "error" && onPause ? (
          <Button variant="ghost" onClick={onPause}>
            {t("mic.pause")}
          </Button>
        ) : null}
      </div>
    </section>
  );
}
