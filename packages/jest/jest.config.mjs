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
    '^react/jsx-runtime$': fixture('./test/cjs/fixtures/fake-jsx-runtime.cjs'),
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

// R10 regression: jsdom + zone.js/testing, target es2016 derived from a
// `<rootDir>`-prefixed tsconfig (the standard jest pattern). Exercises that the
// plugin expands `<rootDir>`, derives the target, and downlevels async so the
// result is the zone-patched global Promise. RED before the <rootDir> fix.
const zone = {
  displayName: 'zone',
  rootDir: '.',
  ...createCjsPreset({ tsconfig: '<rootDir>/test/zone/tsconfig.spec.json' }),
  testEnvironment: 'jsdom',
  testRunner: 'jest-jasmine2',
  testMatch: ['<rootDir>/test/zone/*.spec.ts'],
  setupFilesAfterEnv: ['<rootDir>/test/zone/setup.ts'],
};

export default { projects: [cjs, esm, zone] };
