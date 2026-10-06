"use client";

import { useCallback } from "react";
import { useApi } from "@/api/ApiProvider";
import { ErrorBanner } from "@/components/ErrorBanner";
import type { CosmeticView } from "@/generated/CosmeticView";
import type { GameState } from "@/generated/GameState";
import { dictionaries, type MessageKey } from "@/i18n";
import { Sprite } from "@/sprites/Sprite";
import { useT, type Translate } from "@/state/PreferencesProvider";
import { useResource } from "@/state/useResource";
import { Badge } from "@/ui/Badge";
import { Meter } from "@/ui/Meter";

/** The name of a cosmetic. A cosmetic the dictionaries do not know yet is shown by its id. */
export function cosmeticName(t: Translate, id: string): string {
  const key = `cosmetic.${id}`;
  return key in dictionaries.en ? t(key as MessageKey) : id;
}

function rankName(t: Translate, rank: number): string {
  const clamped = Math.min(Math.max(Math.trunc(rank), 1), 6) as 1 | 2 | 3 | 4 | 5 | 6;
  return t(`game.rank.${clamped}`);
}

function Unlockable({ item }: { item: CosmeticView }) {
  const t = useT();
  return (
    <li className="rows__item" data-locked={!item.unlocked}>
      <div className="px-stack" style={{ "--gap": "var(--space-1)" } as React.CSSProperties}>
        <strong className="rows__text">{cosmeticName(t, item.id)}</strong>
        <span className="px-hint">
          {t(item.kind === "theme" ? "progress.game.unlock.theme" : "progress.game.unlock.accessory")}
          {item.unlocked ? null : ` · ${t("wardrobe.locked", { rank: item.rank })}`}
        </span>
      </div>
      {item.unlocked ? <Badge tone="success">{t(item.equipped ? "progress.game.unlock.equipped" : "progress.game.unlock.unlocked")}</Badge> : <Sprite name="icon-lock" scale={3} label={t("wardrobe.locked", { rank: item.rank })} />}
    </li>
  );
}

function Game({ game }: { game: GameState }) {
  const t = useT();
  const { streak, rank } = game;
  return (
    <div className="px-stack">
      <p>{t("game.note")}</p>
      <p className="px-hint">{t("progress.game.separate")}</p>

      <dl className="facts gamefacts">
        <div>
          <dt>
            <Sprite name="icon-gem" scale={2} /> {t("progress.game.sparks.total")}
          </dt>
          <dd className="gamefacts__number">{game.xp_total}</dd>
        </div>
        <div>
          <dt>{t("game.rank")}</dt>
          <dd>
            <span>{t("game.rank.value", { number: rank.rank, name: rankName(t, rank.rank) })}</span>
            <Meter value={rank.progress_percent} label={t("progress.game.rank.progress")} tone="success" />
            <span className="px-hint">
              {rank.next_threshold === null ? t("progress.game.rank.top") : t("progress.game.rank.next", { xp: rank.next_threshold })}
            </span>
          </dd>
        </div>
        <div>
          <dt>
            <Sprite name="icon-flame" scale={2} /> {t("game.streak")}
          </dt>
          <dd>
            <span>{t("progress.game.streak.current", { count: streak.current })}</span>
            <span className="px-hint">{t("progress.game.streak.longest", { count: streak.longest })}</span>
            <span className="px-hint">{t(streak.active_today ? "progress.game.streak.today.yes" : "progress.game.streak.today.no")}</span>
          </dd>
        </div>
        <div>
          <dt>
            <Sprite name="icon-star" scale={2} /> {t("game.rest")}
          </dt>
          <dd>
            <span>{t("progress.game.rest.tokens", { count: streak.rest_tokens_available })}</span>
            {streak.tokens_needed_to_continue > 0 ? (
              <span className="px-hint">{t("progress.game.rest.needed", { count: streak.tokens_needed_to_continue })}</span>
            ) : null}
          </dd>
        </div>
      </dl>

      <section aria-labelledby="game-source-title" className="px-stack" style={{ "--gap": "var(--space-2)" } as React.CSSProperties}>
        <h3 id="game-source-title">{t("progress.game.source.title")}</h3>
        {game.xp_by_source.length === 0 ? (
          <p>{t("progress.game.xp.empty")}</p>
        ) : (
          <ul className="rows">
            {game.xp_by_source.map((entry) => (
              <li key={entry.source} className="rows__item">
                <span>{t(`progress.kind.${entry.source}`)}</span>
                <strong>{entry.amount}</strong>
              </li>
            ))}
          </ul>
        )}
      </section>

      <section aria-labelledby="game-unlock-title" className="px-stack" style={{ "--gap": "var(--space-2)" } as React.CSSProperties}>
        <h3 id="game-unlock-title">{t("progress.game.unlock.title")}</h3>
        {game.cosmetics.length === 0 ? (
          <p>{t("progress.game.unlock.none")}</p>
        ) : (
          <ul className="rows">
            {game.cosmetics.map((item) => (
              <Unlockable key={item.id} item={item} />
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}

/**
 * The game, kept apart from the assessment on purpose: its own panel, its own
 * heading, its own request (`GET /api/game`), and a sentence that says it is
 * cosmetic. Nothing in it is read from an attempt, a score or an estimate,
 * and it never shows a level.
 */
export function GamePanel() {
  const t = useT();
  const api = useApi();
  const load = useCallback((signal: AbortSignal) => api.getGame(signal), [api]);
  const game = useResource<GameState>(load);
  return (
    <section className="gamepanel" aria-labelledby="game-title" data-kind="game">
      <h2 id="game-title" className="px-panel__title">
        <Sprite name="icon-gem" scale={3} /> {t("progress.game.title")}
      </h2>
      {game.state.status === "loading" ? <p role="status">{t("state.loading")}</p> : null}
      {game.state.status === "error" ? <ErrorBanner error={game.state.error} onRetry={game.reload} /> : null}
      {game.state.status === "ready" ? <Game game={game.state.data} /> : null}
    </section>
  );
}
