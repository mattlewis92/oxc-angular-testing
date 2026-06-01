import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vitest/config';
// Import the plugin source directly — vitest transpiles the config (and this
// import) with esbuild, so tests run against `src/` without a build step.
import oxcAngular from './src/index.ts';

export default defineConfig({
  plugins: [oxcAngular()],
  resolve: {
    alias: {
      // Lightweight fake so the integration test exercises the transform +
      // decorator execution without booting the full Angular runtime.
      '@angular/core': fileURLToPath(
        new URL('./test/fixtures/fake-angular-core.js', import.meta.url),
      ),
    },
  },
  test: {
    include: ['test/**/*.spec.ts'],
    environment: 'node',
  },
});
