import { fileURLToPath } from 'node:url';

const fixture = (p) => fileURLToPath(new URL(p, import.meta.url));

// ESM project: native-ESM jest, `importMode: 'import'`.
const esm = {
  displayName: 'esm',
  rootDir: '.',
  testMatch: ['<rootDir>/test/*.spec.ts'],
  moduleFileExtensions: ['ts', 'mjs', 'js', 'html', 'json'],
  extensionsToTreatAsEsm: ['.ts', '.html'],
  transform: {
    '^.+\\.tsx?$': ['@oxc-angular-testing/jest', { importMode: 'import', esm: true }],
    '^.+\\.html$': '@oxc-angular-testing/jest/html-transformer',
  },
  moduleNameMapper: {
    '^@angular/core$': fixture('./test/fixtures/fake-angular-core.mjs'),
  },
  testEnvironment: 'node',
};

// CommonJS project: classic CJS jest, `importMode: 'require'` — the transform
// emits TS-style `require()` / `exports` with esModuleInterop helpers.
const cjs = {
  displayName: 'cjs',
  rootDir: '.',
  testMatch: ['<rootDir>/test/cjs/*.spec.ts'],
  moduleFileExtensions: ['ts', 'js', 'cjs', 'html', 'json'],
  transform: {
    // `.js` included so ESM-authored dependencies (loaded via jest's CJS loader)
    // are downleveled to CommonJS.
    '^.+\\.(tsx?|js|mjs)$': ['@oxc-angular-testing/jest', { importMode: 'require', esm: false }],
    '^.+\\.html$': '@oxc-angular-testing/jest/html-transformer-cjs',
  },
  moduleNameMapper: {
    '^@angular/core$': fixture('./test/cjs/fixtures/fake-angular-core.cjs'),
  },
  testEnvironment: 'node',
};

export default { projects: [esm, cjs] };
