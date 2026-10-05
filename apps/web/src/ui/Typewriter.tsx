"use client";

import { useEffect, useState } from "react";
import { useMotionEnabled } from "@/state/PreferencesProvider";

export interface TypewriterProps {
  text: string;
  /** Characters revealed per second. */
  speed?: number;
  onDone?: () => void;
}

/**
 * Reveals text a few characters at a time when motion is on. The whole text is
 * always in the page for screen readers, and with motion off it is shown at once.
 */
export function Typewriter({ text, speed = 40, onDone }: TypewriterProps) {
  const motion = useMotionEnabled();
  // The count belongs to one text. A new text starts from zero without any effect.
  const [progress, setProgress] = useState({ text, shown: 0 });
  const shown = !motion ? text.length : progress.text === text ? progress.shown : 0;

  useEffect(() => {
    if (!motion) {
      onDone?.();
      return;
    }
    let count = 0;
    const timer = setInterval(() => {
      count += 1;
      setProgress({ text, shown: count });
      if (count >= text.length) {
        clearInterval(timer);
        onDone?.();
      }
    }, 1000 / speed);
    return () => clearInterval(timer);
    // onDone is deliberately left out: a new callback must not restart the reveal.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [text, motion, speed]);

  return (
    <>
      <span className="sr-only">{text}</span>
      <span aria-hidden="true">{text.slice(0, shown)}</span>
    </>
  );
}
