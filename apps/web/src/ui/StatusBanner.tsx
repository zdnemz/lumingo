import { useId, type ReactNode } from "react";
import { Sprite } from "@/sprites/Sprite";
import type { SpriteName } from "@/sprites/data";
import { Panel, type PanelTone } from "./Panel";

export type BannerTone = "danger" | "warning" | "info" | "success";

const TONE: Record<BannerTone, { panel: PanelTone; icon: SpriteName }> = {
  danger: { panel: "danger", icon: "icon-cross" },
  warning: { panel: "warning", icon: "icon-gear" },
  info: { panel: "primary", icon: "icon-chat" },
  success: { panel: "success", icon: "icon-check" },
};

export interface StatusBannerProps {
  tone: BannerTone;
  title: string;
  /** Text under the title. */
  children?: ReactNode;
  /** The recovery action: buttons or links. A banner that reports a problem always has one. */
  actions?: ReactNode;
}

/**
 * A message about a state the learner can do something about. Danger and
 * warning interrupt a screen reader ("alert"); the others wait their turn.
 * The icon and the title carry the meaning, so colour is never the only cue.
 */
export function StatusBanner({ tone, title, children, actions }: StatusBannerProps) {
  const titleId = useId();
  const { panel, icon } = TONE[tone];
  return (
    <Panel
      tone={panel}
      className="banner"
      role={tone === "danger" || tone === "warning" ? "alert" : "status"}
      aria-labelledby={titleId}
    >
      <div className="px-stack" style={{ "--gap": "var(--space-3)" } as React.CSSProperties}>
        <p className="banner__title" id={titleId}>
          <Sprite name={icon} scale={2} />
          <strong>{title}</strong>
        </p>
        {children}
        {actions ? <div className="px-row">{actions}</div> : null}
      </div>
    </Panel>
  );
}
