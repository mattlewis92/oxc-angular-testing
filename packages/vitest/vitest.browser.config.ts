import { playwright } from '@vitest/browser-playwright';
import { defineConfig } from 'vitest/config';
// Plugin source imported directly — vitest transpiles the config, so the e2e
// runs against `src/` without a build step (same as vitest.config.ts).
import oxcAngular from './src/index.ts';

// Browser-mode e2e: REAL @angular/core (no fake alias) + real scss, driven
// through playwright chromium. `keepStyles` is deliberately NOT set: the
// environment-based default must keep styles here (the `client` environment) —
// the specs fail if it does not, which is the empirical check of the
// environment name vitest browser mode uses.
export default defineConfig({
  plugins: [oxcAngular()],
  test: {
    include: ['test-browser/**/*.spec.ts'],
    browser: {
      enabled: true,
      headless: true,
      provider: playwright(),
      instances: [{ browser: 'chromium' }],
    },
  },
});
