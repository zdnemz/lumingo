"use client";

import { useEffect, useState } from "react";
import { useMotionEnabled } from "@/state/PreferencesProvider";
import type { AccessoryId, LumiMood } from "./data";
import { Sprite } from "./Sprite";

export interface MascotProps {
  mood?: LumiMood;
  /** Something Lumi wears. Cosmetic only. */
  accessory?: AccessoryId;
  scale?: number;
  /** Accessible name. Leave out when the mascot is only decoration. */
  label?: string;
  className?: string;
}

/** How long Lumi stays eyes-open between blinks, and how long a blink lasts. */
const BLINK_GAP_MS = [2600, 5200] as const;
const BLINK_MS = 140;

/**
 * Lumi, the lantern spirit. With motion on, an idle Lumi bobs and blinks now
 * and then. With motion off Lumi is a still picture and the mood is the only
 * thing that changes.
 */
export function Mascot({ mood = "idle", accessory, scale = 6, label, className }: MascotProps) {
  const motion = useMotionEnabled();
  const canBlink = motion && mood === "idle";
  const [blinking, setBlinking] = useState(false);

  useEffect(() => {
    if (!canBlink) return;
    let timer: ReturnType<typeof setTimeout>;
    const schedule = () => {
      const [min, max] = BLINK_GAP_MS;
      timer = setTimeout(() => {
        setBlinking(true);
        timer = setTimeout(() => {
          setBlinking(false);
          schedule();
        }, BLINK_MS);
      }, min + Math.random() * (max - min));
    };
    schedule();
    return () => {
      clearTimeout(timer);
      // Never leave Lumi with closed eyes when blinking stops mid-blink.
      setBlinking(false);
    };
  }, [canBlink]);

  const shown = canBlink && blinking ? "blink" : mood;
  return (
    <span className={["lumi", className].filter(Boolean).join(" ")} data-anim={mood === "idle" || mood === "happy" ? "bob" : undefined}>
      <Sprite name={`lumi-${shown}`} scale={scale} label={label} />
      {accessory ? <Sprite name={accessory} scale={scale} className="lumi__accessory" /> : null}
    </span>
  );
}
