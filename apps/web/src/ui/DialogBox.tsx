import type { ReactNode } from "react";

export interface DialogBoxProps {
  /** Who is speaking, shown on the name tag. */
  name: string;
  children: ReactNode;
  /** Show the "more" arrow in the corner. */
  more?: boolean;
}

/** The tutor's speech box, drawn like a game dialogue window. */
export function DialogBox({ name, children, more }: DialogBoxProps) {
  return (
    <div className="px-dialogbox" role="group" aria-label={name}>
      <span className="px-dialogbox__name" aria-hidden="true">
        {name}
      </span>
      <div>{children}</div>
      {more ? <span className="px-dialogbox__next" data-anim="blink" aria-hidden="true" /> : null}
    </div>
  );
}
