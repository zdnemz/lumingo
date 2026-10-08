"use client";

import type { MessageKey } from "@/i18n";
import { Sprite } from "@/sprites/Sprite";
import { useT } from "@/state/PreferencesProvider";
import { Meter } from "@/ui/Meter";

export interface HudProps {
  /** Consecutive local days with some practice. */
  streak: number;
  sparks: number;
  /** 1 to 6. The server decides the rank, so the thresholds live in one place. */
  rank: number;
  /** Progress to the next rank, 0 to 100. */
  rankProgress: number;
  restTokens: number;
}

const RANK_KEYS: Record<number, MessageKey> = {
  1: "game.rank.1",
  2: "game.rank.2",
  3: "game.rank.3",
  4: "game.rank.4",
  5: "game.rank.5",
  6: "game.rank.6",
};

/**
 * The game strip: streak, sparks and rank. These are rewards for showing up.
 * They are never shown as a level, and the strip says so.
 */
export function Hud({ streak, sparks, rank, rankProgress, restTokens }: HudProps) {
  const t = useT();
  const rankName = t(RANK_KEYS[Math.min(Math.max(rank, 1), 6)] ?? "game.rank.1");
  return (
    <section className="hud" aria-label={t("game.hud")}>
      <p className="hud__item">
        <Sprite name="icon-flame" scale={3} className="hud__flame" />
        <span className="sr-only">{t("game.streak.value", { count: streak })}</span>
        <span className="hud__value" aria-hidden="true">
          {streak}
        </span>
      </p>
      <p className="hud__item">
        <Sprite name="icon-gem" scale={3} className="hud__gem" />
        <span className="sr-only">{t("game.sparks.value", { count: sparks })}</span>
        <span className="hud__value" aria-hidden="true">
          {sparks}
        </span>
      </p>
      <div className="hud__rank" title={t("game.note")}>
        <p className="hud__rank-name">{t("game.rank.value", { number: rank, name: rankName })}</p>
        <Meter value={rankProgress} label={t("game.rank")} tone="success" />
      </div>
      {restTokens > 0 ? (
        <p className="hud__item hud__rest">
          <Sprite name="icon-star" scale={2} />
          <span>{t("game.rest", { count: restTokens })}</span>
        </p>
      ) : null}
    </section>
  );
}
