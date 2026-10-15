import type { E2EConfig } from 'e2e';
import { web } from '@e2e-dev/web';
import { opencodeConsole } from 'e2e/oauth/opencode-console';

// Set APP_URL to test a server the developer started themselves: the runner
// then drives that address and starts nothing of its own. Without it the run
// asks for a free port and starts `scripts/e2e-server.sh` there, which builds
// and runs the offline server, so `pnpm test:e2e` needs no setup at all.
const appUrl = process.env.APP_URL;

export default {
  // One worker, because the app holds one session at a time: `SessionManager::start`
  // refuses a second session while one runs with `Conflict`, so two specs that
  // drive a session in parallel would fail each other. One worker also means one
  // cold build per run instead of one per worker.
  workers: 1,
  // Your OpenCode Console workspace serves the model; sign in once with `e2e login opencode-console`, `e2e models opencode-console` lists the ids.
  agents: {
    default: {
      model: opencodeConsole('deepseek-v4.1-flash'),
      system: 'You are a thorough QA agent. Verify every outcome.',
    },
  },
  targets: [{
    engine: web(),
    app: {
      url: appUrl ?? 'http://127.0.0.1:0',
      // The offline suite runs no agent step, so it needs no model key.
      ...(appUrl === undefined ? {
        command: {
          executable: 'sh',
          args: ['scripts/e2e-server.sh', '{port}'],
          // A cold run builds the web export and the whole Rust workspace.
          startupTimeout: 1_800_000,
          log: '.e2e/logs/server.log',
        },
      } : {}),
    },
  }],
} satisfies E2EConfig;
