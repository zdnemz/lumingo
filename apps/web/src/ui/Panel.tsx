import type { ElementType, HTMLAttributes, ReactNode } from "react";
import { cx } from "./cx";

export type PanelTone = "listening" | "speaking" | "reading" | "writing" | "success" | "danger" | "warning" | "primary";

export interface PanelProps extends Omit<HTMLAttributes<HTMLElement>, "title"> {
  as?: ElementType;
  title?: ReactNode;
  tone?: PanelTone;
  raised?: boolean;
  inset?: boolean;
}

export function Panel({ as: Tag = "section", title, tone, raised, inset, className, children, ...rest }: PanelProps) {
  return (
    <Tag
      {...rest}
      data-tone={tone}
      className={cx("px-panel", raised && "px-panel--raised", inset && "px-panel--inset", className)}
    >
      {title ? <h2 className="px-panel__title">{title}</h2> : null}
      {children}
    </Tag>
  );
}
