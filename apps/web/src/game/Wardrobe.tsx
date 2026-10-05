"use client";

import { Mascot } from "@/sprites/Mascot";
import type { AccessoryId } from "@/sprites/data";
import { useT } from "@/state/PreferencesProvider";
import { Button } from "@/ui/Button";
import { Panel } from "@/ui/Panel";

export type ThemeCosmeticId = "theme-forest" | "theme-ember";
export type CosmeticId = AccessoryId | ThemeCosmeticId;

export interface CosmeticItem {
  id: CosmeticId;
  kind: "theme" | "accessory";
  /** The rank at which it unlocks. */
  rank: number;
  unlocked: boolean;
  equipped: boolean;
}

export interface WardrobeProps {
  items: readonly CosmeticItem[];
  /** The learner chose to wear or use an unlocked item. */
  onEquip: (id: CosmeticId) => void;
  /** The learner took an accessory off. Themes cannot be taken off, only swapped. */
  onUnequip: (id: CosmeticId) => void;
}

function themeOf(id: ThemeCosmeticId): "forest" | "ember" {
  return id === "theme-forest" ? "forest" : "ember";
}

function Preview({ item }: { item: CosmeticItem }) {
  if (item.kind === "accessory") {
    return <Mascot mood="happy" accessory={item.id as AccessoryId} scale={5} />;
  }
  // A small window painted with the theme's own colours.
  return (
    <span className="wardrobe__swatch" data-theme-scope={themeOf(item.id as ThemeCosmeticId)} aria-hidden="true">
      <span className="wardrobe__swatch-surface" />
      <span className="wardrobe__swatch-primary" />
    </span>
  );
}

/** Rewards the learner has earned and the ones still ahead. Cosmetic only, and the page says so. */
export function Wardrobe({ items, onEquip, onUnequip }: WardrobeProps) {
  const t = useT();
  return (
    <section className="wardrobe" aria-labelledby="wardrobe-title">
      <h2 id="wardrobe-title">{t("wardrobe.title")}</h2>
      <p>{t("wardrobe.intro")}</p>
      <ul className="wardrobe__grid">
        {items.map((item) => {
          const name = t(`cosmetic.${item.id}`);
          const isTheme = item.kind === "theme";
          return (
            <li key={item.id}>
              <Panel as="div" tone={item.equipped ? "primary" : undefined} className="wardrobe__card" data-locked={!item.unlocked}>
                <div className="wardrobe__preview" style={item.unlocked ? undefined : { opacity: 0.45 }}>
                  <Preview item={item} />
                </div>
                <p className="wardrobe__name">{name}</p>
                {item.unlocked ? (
                  item.equipped && isTheme ? (
                    <p className="wardrobe__state">{t("wardrobe.using")}</p>
                  ) : item.equipped ? (
                    <Button small onClick={() => onUnequip(item.id)} aria-label={`${t("wardrobe.takeoff")}: ${name}`}>
                      {t("wardrobe.takeoff")}
                    </Button>
                  ) : (
                    <Button
                      small
                      variant="primary"
                      onClick={() => onEquip(item.id)}
                      aria-label={`${isTheme ? t("wardrobe.use") : t("wardrobe.wear")}: ${name}`}
                    >
                      {isTheme ? t("wardrobe.use") : t("wardrobe.wear")}
                    </Button>
                  )
                ) : (
                  <p className="wardrobe__state">{t("wardrobe.locked", { rank: item.rank })}</p>
                )}
              </Panel>
            </li>
          );
        })}
      </ul>
    </section>
  );
}
