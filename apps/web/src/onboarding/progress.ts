// Where the setup wizard is, kept in this browser so a reloaded tab returns to
// the right step. The program has no "setup finished" flag of its own, so this
// is a convenience of this browser only: clearing site data starts the wizard
// again, and nothing here is needed for the program to work.

export const ONBOARDING_KEY = "lumingo.onboarding.v1";

export const STEPS = ["welcome", "language", "privacy", "guide", "provider", "test", "hardware", "ready"] as const;
export type StepId = (typeof STEPS)[number];

export interface OnboardingProgress {
  step: StepId;
  /** The learner reached the end once. */
  done: boolean;
}

export function isStep(value: unknown): value is StepId {
  return typeof value === "string" && (STEPS as readonly string[]).includes(value);
}

/** Reads stored progress. Anything missing or damaged is "never started". */
export function parseProgress(raw: string | null): OnboardingProgress | null {
  if (raw === null) return null;
  try {
    const value: unknown = JSON.parse(raw);
    if (typeof value !== "object" || value === null) return null;
    const record = value as Record<string, unknown>;
    if (!isStep(record.step)) return null;
    return { step: record.step, done: record.done === true };
  } catch {
    return null;
  }
}

export function loadProgress(win: Window): OnboardingProgress | null {
  try {
    return parseProgress(win.localStorage.getItem(ONBOARDING_KEY));
  } catch {
    return null;
  }
}

export function saveProgress(win: Window, progress: OnboardingProgress): void {
  try {
    win.localStorage.setItem(ONBOARDING_KEY, JSON.stringify(progress));
  } catch {
    // Blocked or full: the wizard still works for this visit.
  }
}

/** What the program knows that limits where the wizard may stand. */
export interface StepContext {
  hasProvider: boolean;
}

/**
 * The step to show after a reload. The stored step is trusted only as far as
 * the program's own state allows: the test and the steps after it need a
 * provider profile, so without one the wizard returns to the provider step.
 * A learner who finished before starts again at the welcome.
 */
export function startStep(progress: OnboardingProgress | null, context: StepContext): StepId {
  if (progress === null || progress.done) return "welcome";
  if (!context.hasProvider && STEPS.indexOf(progress.step) > STEPS.indexOf("provider")) return "provider";
  return progress.step;
}

export function stepIndex(step: StepId): number {
  return STEPS.indexOf(step);
}
