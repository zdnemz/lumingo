"use client";

import { AppShell } from "@/components/AppShell";
import { OnboardingWizard } from "@/onboarding/OnboardingWizard";

export default function OnboardingPage() {
  return (
    <AppShell>
      <OnboardingWizard />
    </AppShell>
  );
}
