import type { CSSProperties } from "react";
import { SPRITES, type SpriteName } from "./data";

export interface SpriteProps {
  name: SpriteName;
  /** CSS pixels per sprite pixel. Whole numbers keep the edges crisp. */
  scale?: number;
  /** Accessible name. Without one the sprite is decoration and is hidden from screen readers. */
  label?: string;
  className?: string;
  style?: CSSProperties;
}

interface Run {
  x: number;
  y: number;
  length: number;
  colour: string;
}

/** Merges horizontal runs of the same colour so a sprite is a handful of rectangles. */
export function runsOf(rows: readonly string[]): Run[] {
  const runs: Run[] = [];
  rows.forEach((row, y) => {
    let x = 0;
    while (x < row.length) {
      const colour = row.charAt(x);
      if (colour === ".") {
        x += 1;
        continue;
      }
      let end = x + 1;
      while (end < row.length && row.charAt(end) === colour) end += 1;
      runs.push({ x, y, length: end - x, colour });
      x = end;
    }
  });
  return runs;
}

export function Sprite({ name, scale = 2, label, className, style }: SpriteProps) {
  const def = SPRITES[name];
  const height = def.rows.length;
  const width = def.rows[0]?.length ?? 0;
  const runs = runsOf(def.rows);
  const accessibility = label ? { role: "img", "aria-label": label } : { "aria-hidden": true };
  return (
    <svg
      className={["px-sprite", className].filter(Boolean).join(" ")}
      viewBox={`0 0 ${width} ${height}`}
      width={width * scale}
      height={height * scale}
      focusable="false"
      style={style}
      {...accessibility}
    >
      {runs.map((run) => (
        <rect
          key={`${run.y}-${run.x}`}
          className={`sp-${run.colour}`}
          x={run.x}
          y={run.y}
          width={run.length}
          height={1}
        />
      ))}
    </svg>
  );
}
