"use client";

import { Sprite } from "@/sprites/Sprite";
import { useT } from "@/state/PreferencesProvider";
import { Button } from "@/ui/Button";

export interface ListenButtonProps {
  onListen: () => void;
  /** Plays still allowed, or null when replays are unlimited. */
  playsLeft: number | null;
  playing: boolean;
}

/**
 * Asks the server to play a text through the speakers. The browser plays no
 * sound: the program on the learner's computer does.
 */
export function ListenButton({ onListen, playsLeft, playing }: ListenButtonProps) {
  const t = useT();
  const exhausted = playsLeft !== null && playsLeft <= 0;
  return (
    <div className="px-row">
      <Button onClick={onListen} disabled={exhausted || playing} busy={playing}>
        <Sprite name="icon-speaker" scale={2} />
        {playing ? t("activity.listen.playing") : t("activity.listen")}
      </Button>
      {playsLeft !== null ? <span className="px-hint">{t("activity.listen.left", { count: playsLeft })}</span> : null}
    </div>
  );
}
