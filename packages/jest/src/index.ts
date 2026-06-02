import * as crypto from 'node:crypto';
import * as path from 'node:path';
import { transform, type TransformOptions } from '@oxc-angular-testing/transform';
import {
  deriveTransformOptions,
  type DerivedTransformOptions,
} from '@oxc-angular-testing/transform/tsconfig';
import { version as transformVersion } from '@oxc-angular-testing/transform/package.json';

/**
 * Expand a leading jest `<rootDir>` token. Jest expands `<rootDir>` in its own
 * well-known config fields but NOT inside a transformer's option object, so a
 * `{ tsconfig: '<rootDir>/tsconfig.json' }` (the standard jest pattern) arrives
 * here unexpanded. We expand it ourselves against the project `rootDir`. Any
 * path without the token is returned unchanged.
 */
function expandRootDir(p: string, rootDir: string | undefined): string {
  if (rootDir && p.startsWith('<rootDir>')) {
    return path.join(rootDir, p.slice('<rootDir>'.length));
  }
  return p;
}

/** The per-file `options` jest passes to a transformer (the slice we read). */
interface JestTransformOptions {
  instrument?: boolean;
  config?: { rootDir?: string; cwd?: string };
}

export interface OxcAngularJestOptions {
  /**
   * Output module format: `"commonjs"` (default) or `"esm"`. Drives `templateUrl`
   * (`require` vs top-level `import`) and the ESM→CommonJS rewrite. Derived from
   * the tsconfig `module` when a `tsconfig` is given; the CJS/ESM presets set it.
   */
  module?: 'commonjs' | 'esm';
  /** Always instrument for coverage (jest also enables this when collecting coverage). */
  coverage?: boolean;
  /** Path to a tsconfig to derive target / module / decorator flags from. */
  tsconfig?: string;
  /** Extra regex sources marking files as ESM dependencies to downlevel. */
  processEsmModules?: string[];
  /**
   * Files whose path matches this regex are returned as a string module (their
   * raw content) instead of being compiled — for component `templateUrl` HTML
   * and inline SVG. Mirrors jest-preset-angular's `stringifyContentPathRegex`.
   * Default: `\\.(html|svg)$`. Set to `null`/`''` to disable.
   */
  stringifyContentPathRegex?: string | null;
  /** Override individual transform options forwarded to the Rust transform. */
  transform?: Partial<Omit<TransformOptions, 'module'>>;
}

/** Minimal structural shape of a synchronous jest transformer. */
export interface JestSyncTransformer {
  canInstrument: boolean;
  getCacheKey(
    sourceText: string,
    sourcePath: string,
    options: JestTransformOptions,
  ): string;
  process(
    sourceText: string,
    sourcePath: string,
    options?: JestTransformOptions,
  ): { code: string; map?: unknown };
}

/**
 * Decide whether a file is an ESM dependency that should be downleveled to the
 * runner's module format with the Angular/TS passes skipped — i.e. a `.mjs`
 * file or anything under `node_modules` (e.g. `@angular/core`, which ships only
 * ESM and must be converted to CommonJS for classic CJS jest). Additional regex
 * sources can be supplied via `processEsmModules`.
 *
 * Mirrors jest-preset-angular's `processWithEsbuild` fast path, but uses our own
 * oxc ESM→CJS transform instead of esbuild.
 */
export function isEsmDependency(
  sourcePath: string,
  processEsmModules?: string[],
): boolean {
  if (sourcePath.endsWith('.mjs') || sourcePath.includes('/node_modules/')) {
    return true;
  }
  if (Array.isArray(processEsmModules)) {
    return processEsmModules.some((pattern) => new RegExp(pattern).test(sourcePath));
  }
  return false;
}

/**
 * Jest transformer factory. Wire up in jest config (or use `createCjsPreset` /
 * `createEsmPreset` from `@oxc-angular-testing/jest/presets`):
 *
 *   transform: { '^.+\\.(ts|js|mjs)$': ['@oxc-angular-testing/jest', { module: 'commonjs' }] }
 *
 * Works under both classic CommonJS jest (`module: 'commonjs'`, the default) and
 * native-ESM jest (`module: 'esm'` + Node vm modules). CommonJS output matches
 * TypeScript's `module: "commonjs"` + `esModuleInterop` emit. ESM-only
 * dependencies (`.mjs` / `node_modules`) are downleveled to the runner's module
 * format with the Angular passes skipped.
 */
export function createTransformer(
  transformerOptions: OxcAngularJestOptions = {},
): JestSyncTransformer {
  // `null`/`''` disables stringification. (Independent of the tsconfig.)
  const stringifyPattern =
    transformerOptions.stringifyContentPathRegex === undefined
      ? '\\.(html|svg)$'
      : transformerOptions.stringifyContentPathRegex;
  const stringifyRe = stringifyPattern ? new RegExp(stringifyPattern) : null;

  // tsconfig derivation is DEFERRED to first use. The project `rootDir` needed to
  // expand a `<rootDir>/tsconfig.json` option is only available per-file on
  // `options.config` — not at factory time. Deriving eagerly here would read the
  // literal (unexpanded) path, find nothing, and derive NO options — silently
  // dropping `target` (so oxc defaults to esnext: `async` stays native, etc.),
  // `module`, and the decorator flags. Resolve lazily and memoize.
  let resolved:
    | { derived: DerivedTransformOptions; moduleKind: 'commonjs' | 'esm'; esmOutput: boolean }
    | undefined;
  const resolve = (config?: { rootDir?: string; cwd?: string }) => {
    if (resolved) return resolved;
    let derived: DerivedTransformOptions = {};
    if (transformerOptions.tsconfig) {
      const rootDir = config?.rootDir;
      const tsconfigPath = expandRootDir(transformerOptions.tsconfig, rootDir);
      derived = deriveTransformOptions(tsconfigPath, rootDir ?? config?.cwd);
    }
    const moduleKind = transformerOptions.module ?? derived.module ?? 'commonjs';
    // ESM output controls the stringified content module form (export vs module.exports).
    resolved = { derived, moduleKind, esmOutput: moduleKind === 'esm' };
    return resolved;
  };

  return {
    canInstrument: true,
    // Include the native transform version so jest's transform cache is
    // invalidated when the binding (and thus its output) changes.
    getCacheKey(sourceText, sourcePath, options) {
      const { derived } = resolve(options?.config);
      return crypto
        .createHash('sha1')
        .update(transformVersion)
        .update('\0')
        .update(JSON.stringify(transformerOptions))
        .update('\0')
        .update(JSON.stringify(derived))
        .update('\0')
        .update(options?.instrument ? '1' : '0')
        .update('\0')
        .update(sourcePath)
        .update('\0')
        .update(sourceText)
        .digest('hex');
    },
    process(sourceText, sourcePath, options) {
      const { derived, moduleKind, esmOutput } = resolve(options?.config);
      // Component templateUrl HTML / inline SVG: return the raw content as a
      // string module rather than compiling it (which would parse `<svg>` etc.
      // as code).
      if (stringifyRe?.test(sourcePath)) {
        const literal = JSON.stringify(sourceText);
        return {
          code: esmOutput ? `export default ${literal};` : `module.exports = ${literal};`,
        };
      }
      const collectCoverage = Boolean(options?.instrument);
      const isDep = isEsmDependency(sourcePath, transformerOptions.processEsmModules);
      const opts: TransformOptions = {
        ...derived,
        ...transformerOptions.transform,
        module: moduleKind,
        coverage: collectCoverage || Boolean(transformerOptions.coverage),
        // Hoist `jest.mock()` above imports for the user's test code (jest
        // requires this). Deps are library code with no jest.mock + skip JIT.
        ...(isDep
          ? { jitTransforms: false, hoistJestMock: false }
          : { hoistJestMock: true }),
      };
      const out = transform(sourceText, sourcePath, opts);
      if (out.errors && out.errors.length > 0) {
        throw new Error(`@oxc-angular-testing/jest: ${out.errors.join('\n')}`);
      }
      return { code: out.code, map: out.map ? JSON.parse(out.map) : undefined };
    },
  };
}

// jest's transformer loader unwraps the module's default export
// (`interopRequireDefault(...).default`), so expose the factory as the default
// too (named exports remain for TypeScript consumers).
export default { createTransformer, isEsmDependency };
