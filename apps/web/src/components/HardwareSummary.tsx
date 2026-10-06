"use client";

import type { HardwareProfile } from "@/generated/HardwareProfile";
import { useT } from "@/state/PreferencesProvider";
import { Sprite } from "@/sprites/Sprite";

/** Decimal gigabytes with one decimal at most, as the floor in the requirements is written. */
function gigabytes(bytes: number): string {
  const value = Math.round((bytes / 1_000_000_000) * 10) / 10;
  return Number.isInteger(value) ? String(value) : value.toFixed(1);
}

/** What the program measured about this computer, and whether it reaches the minimum. Nothing is guessed. */
export function HardwareSummary({ hardware }: { hardware: HardwareProfile }) {
  const t = useT();
  const unknown = t("hardware.unknown");
  const minimum = {
    ram: t("hardware.gb", { count: gigabytes(hardware.minimum_ram_bytes) }),
    cores: hardware.minimum_logical_cores,
  };
  const verdict =
    hardware.meets_minimum === null
      ? t("hardware.meets.unknown", minimum)
      : hardware.meets_minimum
        ? t("hardware.meets.yes", minimum)
        : t("hardware.meets.no", minimum);
  return (
    <div className="px-stack" style={{ "--gap": "var(--space-3)" } as React.CSSProperties}>
      <dl className="facts">
        <div>
          <dt>{t("hardware.ram")}</dt>
          <dd>{hardware.ram_total_bytes === null ? unknown : t("hardware.gb", { count: gigabytes(hardware.ram_total_bytes) })}</dd>
        </div>
        <div>
          <dt>{t("hardware.cores")}</dt>
          <dd>{hardware.logical_cores === null ? unknown : hardware.logical_cores}</dd>
        </div>
        <div>
          <dt>{t("hardware.minimum")}</dt>
          <dd>
            {minimum.ram}, {minimum.cores}
          </dd>
        </div>
      </dl>
      <p className="px-row">
        <Sprite name={hardware.meets_minimum === false ? "icon-cross" : hardware.meets_minimum ? "icon-check" : "icon-gear"} scale={3} />
        <span>{verdict}</span>
      </p>
    </div>
  );
}
