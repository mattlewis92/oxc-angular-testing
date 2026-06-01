import { fileURLToPath } from 'node:url';
import { createCjsPreset, createEsmPreset } from '@oxc-angular-testing/jest/presets';

const fixture = (p) => fileURLToPath(new URL(p, import.meta.url));

// Classic CommonJS project (run without --experimental-vm-modules).
const cjs = {
  displayName: 'cjs',
  rootDir: '.',
  testMatch: ['<rootDir>/test/cjs/*.spec.ts'],
  ...createCjsPreset(),
  moduleNameMapper: {
    '^@angular/core$': fixture('./test/cjs/fixtures/fake-angular-core.cjs'),
  },
};

// Native-ESM project (run with --experimental-vm-modules).
const esm = {
  displayName: 'esm',
  rootDir: '.',
  testMatch: ['<rootDir>/test/*.spec.ts'],
  ...createEsmPreset(),
  moduleNameMapper: {
    '^@angular/core$': fixture('./test/fixtures/fake-angular-core.mjs'),
  },
};

export default { projects: [cjs, esm] };
