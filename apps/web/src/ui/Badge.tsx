import type { ReactNode } from "react";

export interface BadgeProps {
  tone?: "estimate" | "success" | "danger" | "info";
  children: ReactNode;
}

export function Badge({ tone, children }: BadgeProps) {
  return (
    <span className="px-badge" data-tone={tone}>
      {children}
    </span>
  );
}
