// Ready-made jest config presets, mirroring jest-preset-angular's
// createCjsPreset / createEsmPreset. Spread into your jest config:
//
//   import { createCjsPreset } from '@oxc-angular-testing/jest/presets';
//   export default { ...createCjsPreset({ tsconfig: './tsconfig.spec.json' }) };

import type { OxcAngularJestOptions } from './index.js';

const TRANSFORMER = '@oxc-angular-testing/jest';

export interface JestPresetConfig {
  moduleFileExtensions: string[];
  transform: Record<string, string | [string, Record<string, unknown>]>;
  transformIgnorePatterns: string[];
  testEnvironment: string;
  extensionsToTreatAsEsm?: string[];
}

/**
 * Classic CommonJS jest. TypeScript and ESM-only `.mjs` / `node_modules`
 * dependencies (e.g. `@angular/core`) are downleveled to CommonJS. Run *without*
 * `--experimental-vm-modules` so jest loads the transformed `.mjs` as CommonJS.
 */
export function createCjsPreset(
  transformerOptions: OxcAngularJestOptions = {},
): JestPresetConfig {
  const opts = { importMode: 'require', esm: false, ...transformerOptions };
  return {
    moduleFileExtensions: ['ts', 'tsx', 'js', 'mjs', 'html', 'svg', 'json'],
    transform: {
      // One transformer for code + content: `.html`/`.svg` are stringified
      // (via stringifyContentPathRegex), the rest compiled.
      '^.+\\.(ts|tsx|js|mjs|html|svg)$': [TRANSFORMER, opts],
    },
    // Transform `.mjs` files in node_modules (e.g. @angular/*), ignore the rest.
    transformIgnorePatterns: ['node_modules/(?!.*\\.mjs$)'],
    testEnvironment: 'node',
  };
}

/**
 * Native-ESM jest (run with `NODE_OPTIONS=--experimental-vm-modules`).
 * TypeScript is emitted as ESM; `.mjs` / `node_modules` are loaded natively.
 */
export function createEsmPreset(
  transformerOptions: OxcAngularJestOptions = {},
): JestPresetConfig {
  const opts = { importMode: 'import', esm: true, ...transformerOptions };
  return {
    moduleFileExtensions: ['ts', 'tsx', 'mjs', 'js', 'html', 'svg', 'json'],
    extensionsToTreatAsEsm: ['.ts', '.tsx', '.html', '.svg'],
    transform: {
      '^.+\\.(ts|tsx|html|svg)$': [TRANSFORMER, opts],
    },
    transformIgnorePatterns: ['node_modules/(?!tslib)'],
    testEnvironment: 'node',
  };
}
