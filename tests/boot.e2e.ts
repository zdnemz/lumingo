import { test } from '@e2e-dev/web';
import { expect } from 'e2e';

// The harness end to end: `pnpm test:e2e` starts the offline server itself, so
// this spec proves the UI boots against it. The server runs from a fresh
// temporary working directory with a fresh data directory, which means no
// provider and no local progress, and that is exactly the first-run case the
// gate in `apps/web/src/onboarding/FirstRunGate.tsx` handles: a visitor in that
// state is moved to the setup wizard.
test('the offline server serves the UI and a first run lands on the setup wizard', async ({ app, browser, screen }) => {
  await app.open('/');
  await expect(browser).toHaveURL('/onboarding/', { timeout: 15_000 });

  // The wizard's first screen. The heading is the wizard's own title, so it is
  // visible only once the provider list has loaded and the welcome step shows.
  const heading = screen.getByRole('heading', { level: 1, name: /Welcome to Lumingo|Selamat datang di Lumingo/ });
  await expect(heading).toBeVisible();
});
