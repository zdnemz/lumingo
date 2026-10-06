"use client";

import { useRouter } from "next/navigation";
import { useEffect } from "react";
import { useServerState } from "@/api/ServerState";
import { loadProgress } from "./progress";

/**
 * Sends a learner who has never opened the setup, and has no provider, to the
 * setup (journey J1). Someone who already has a provider, for example from a
 * .env file, is not moved: the banner and Settings are enough for them.
 */
export function FirstRunGate() {
  const router = useRouter();
  const { snapshot } = useServerState();
  const needsSetup = snapshot !== null && snapshot.provider === null;
  useEffect(() => {
    if (needsSetup && loadProgress(window) === null) router.replace("/onboarding/");
  }, [needsSetup, router]);
  return null;
}
