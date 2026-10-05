import { cx } from "./cx";

export interface MeterProps {
  /** 0 to 100. */
  value: number;
  /** What the bar measures, for screen readers. */
  label: string;
  tone?: "listening" | "speaking" | "reading" | "writing" | "success" | "danger";
  /** Pulse while a value is changing live, such as the microphone level. */
  live?: boolean;
  className?: string;
}

export function Meter({ value, label, tone, live, className }: MeterProps) {
  const clamped = Math.max(0, Math.min(100, Math.round(value)));
  return (
    <div
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={clamped}
      data-tone={tone}
      data-live={live ? "true" : undefined}
      className={cx("px-meter", className)}
      style={{ "--value": clamped } as React.CSSProperties}
    >
      <div className="px-meter__fill" />
    </div>
  );
}
