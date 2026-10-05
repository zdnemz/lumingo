"use client";

import Link from "next/link";
import { Mascot } from "@/sprites/Mascot";
import { Sprite } from "@/sprites/Sprite";
import { useT } from "@/state/PreferencesProvider";

export type NodeState = "locked" | "open" | "current" | "done";

export interface QuestNode {
  id: string;
  /** 1-based number inside its region. */
  number: number;
  title: string;
  state: NodeState;
  /** How many of the four skills have practice in this unit, 0 to 4. */
  skillsPractised: number;
}

export interface QuestRegion {
  /** CEFR scale name, for example "A1". It names a region of the map; it is not an estimate. */
  level: string;
  nodes: readonly QuestNode[];
}

/** One route per unit. A static export reads the id from the query string. */
export function unitHref(id: string): string {
  return `/unit/?id=${encodeURIComponent(id)}`;
}

function NodeBody({ node }: { node: QuestNode }) {
  const t = useT();
  const stateText = t(`quest.state.${node.state}`);
  return (
    <>
      <span className="quest-node__orb" data-state={node.state} data-anim={node.state === "current" ? "glow" : undefined}>
        {node.state === "locked" ? <Sprite name="icon-lock" scale={3} /> : null}
        {node.state === "done" ? <Sprite name="icon-check" scale={3} /> : null}
        {node.state === "current" ? <Mascot mood="happy" scale={3} /> : null}
        {node.state === "open" ? <span className="quest-node__num">{node.number}</span> : null}
      </span>
      <span className="quest-node__title">{node.title}</span>
      <span className="sr-only">
        {stateText}. {t("quest.skills.done", { count: node.skillsPractised })}
      </span>
      {node.state !== "locked" ? (
        <span className="quest-node__pips" aria-hidden="true">
          {[0, 1, 2, 3].map((index) => (
            <span key={index} className="quest-node__pip" data-on={index < node.skillsPractised} />
          ))}
        </span>
      ) : null}
    </>
  );
}

/** The map of units. Locked units are plain text, not links, so keyboard users never land on a dead end. */
export function QuestMap({ regions }: { regions: readonly QuestRegion[] }) {
  const t = useT();
  return (
    <div className="quest" aria-label={t("quest.map")} role="region">
      {regions.map((region) => (
        <section key={region.level} className="quest-region" aria-labelledby={`region-${region.level}`}>
          <h2 id={`region-${region.level}`} className="quest-region__banner">
            {t("quest.region", { level: region.level })}
          </h2>
          <ol className="quest-trail" data-stagger="">
            {region.nodes.map((node, index) => {
              const label = t("quest.unit", { number: node.number, title: node.title });
              return (
                <li key={node.id} className="quest-node" style={{ "--i": index } as React.CSSProperties}>
                  {node.state === "locked" ? (
                    <span className="quest-node__link" aria-disabled="true" aria-label={label}>
                      <NodeBody node={node} />
                    </span>
                  ) : (
                    <Link
                      href={unitHref(node.id)}
                      className="quest-node__link"
                      aria-label={label}
                      aria-current={node.state === "current" ? "step" : undefined}
                    >
                      <NodeBody node={node} />
                    </Link>
                  )}
                </li>
              );
            })}
          </ol>
        </section>
      ))}
    </div>
  );
}
