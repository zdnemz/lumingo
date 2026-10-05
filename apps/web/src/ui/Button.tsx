import Link from "next/link";
import type { ButtonHTMLAttributes, ReactNode } from "react";
import { cx } from "./cx";

export type ButtonVariant = "default" | "primary" | "danger" | "ghost";

interface CommonProps {
  variant?: ButtonVariant;
  small?: boolean;
  block?: boolean;
  children: ReactNode;
}

function classes({ variant = "default", small, block }: CommonProps): string {
  return cx(
    "px-btn",
    variant !== "default" && `px-btn--${variant}`,
    small && "px-btn--small",
    block && "px-btn--block",
  );
}

export interface ButtonProps extends CommonProps, Omit<ButtonHTMLAttributes<HTMLButtonElement>, "children"> {
  /** While true the button ignores clicks and says so to assistive technology. */
  busy?: boolean;
}

export function Button({ variant, small, block, busy, className, children, onClick, ...rest }: ButtonProps) {
  return (
    <button
      type="button"
      {...rest}
      className={cx(classes({ variant, small, block, children }), className)}
      aria-busy={busy || undefined}
      aria-disabled={busy || rest.disabled || undefined}
      onClick={(event) => {
        if (busy) {
          event.preventDefault();
          return;
        }
        onClick?.(event);
      }}
    >
      {children}
    </button>
  );
}

export interface LinkButtonProps extends CommonProps {
  href: string;
  className?: string;
}

/** A link that looks like a button. Navigation is a link, never a button with an onClick. */
export function LinkButton({ href, variant, small, block, className, children }: LinkButtonProps) {
  return (
    <Link href={href} className={cx(classes({ variant, small, block, children }), className)}>
      {children}
    </Link>
  );
}
