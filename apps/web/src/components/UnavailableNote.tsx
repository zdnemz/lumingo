"use client";

import { useServerState } from "@/api/ServerState";
import type { Feature } from "@/generated/Feature";
import type { MessageKey } from "@/i18n";
import { Sprite } from "@/sprites/Sprite";
import { useT } from "@/state/PreferencesProvider";

/** A part of the program that is not built yet, or a route without a control on this screen. */
export type Missing = Feature | "inspector";

const FEATURE_TEXT: Record<Missing, MessageKey> = {
  sessions: "feature.sessions",
  activities: "feature.activities",
  free_modes: "feature.free_modes",
  speech: "feature.speech",
  models: "feature.models",
  inspector: "feature.inspector",
};

/**
 * What the program reports as missing. `null` until the first snapshot has
 * arrived, so a screen never claims a feature exists before it knows.
 */
export function useUnavailable(): readonly Feature[] | null {
  return useServerState().snapshot?.unavailable ?? null;
}

/** True when the snapshot lists the feature as missing. False while nothing is known. */
export function useIsUnavailable(feature: Feature): boolean {
  return useUnavailable()?.includes(feature) ?? false;
}

export interface UnavailableNoteProps {
  feature: Missing;
}

/**
 * The honest state of a screen part whose route is missing from this build.
 * It is text only: no disabled control pretends to work. The inspector has no
 * entry in the snapshot's list because the program has no route for it at all,
 * so it always shows the note.
 */
export function UnavailableNote({ feature }: UnavailableNoteProps) {
  const t = useT();
  const unavailable = useUnavailable();
  const name = t(FEATURE_TEXT[feature]);
  if (feature === "inspector") {
    return <Note title={t("unavailable.title")} body={t("unavailable.inspector")} />;
  }
  if (unavailable === null) return <p className="px-hint">{t("state.loading")}</p>;
  // The program now includes the feature, but this screen was built before it
  // did. Saying so is better than hiding the gap or showing a fake control.
  const body = unavailable.includes(feature) ? t("unavailable.body", { feature: name }) : t("unavailable.noui", { feature: name });
  return <Note title={t("unavailable.title")} body={body} />;
}

function Note({ title, body }: { title: string; body: string }) {
  return (
    <div className="unavailable" role="note">
      <p className="unavailable__title">
        <Sprite name="icon-lock" scale={2} />
        <strong>{title}</strong>
      </p>
      <p className="px-hint">{body}</p>
    </div>
  );
}
