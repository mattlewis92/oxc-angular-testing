import * as crypto from 'node:crypto';
import { transform, type TransformOptions } from '@oxc-angular-testing/transform';
import { deriveTransformOptions } from '@oxc-angular-testing/transform/tsconfig';
import { version as transformVersion } from '@oxc-angular-testing/transform/package.json';

export interface OxcAngularJestOptions {
  /** `"auto"` (default), `"require"`, or `"import"`. */
  importMode?: 'auto' | 'require' | 'import';
  /** Whether the project's module kind is ESM (used when importMode is auto). */
  esm?: boolean;
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
  transform?: Partial<TransformOptions>;
}

/** Minimal structural shape of a synchronous jest transformer. */
export interface JestSyncTransformer {
  canInstrument: boolean;
  getCacheKey(
    sourceText: string,
    sourcePath: string,
    options: { instrument?: boolean },
  ): string;
  process(
    sourceText: string,
    sourcePath: string,
    options?: { instrument?: boolean },
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
 *   transform: { '^.+\\.(ts|js|mjs)$': ['@oxc-angular-testing/jest', { importMode: 'require' }] }
 *
 * Works under both classic CommonJS jest (`importMode: 'require'`; the default
 * `'auto'` resolves to require for CJS) and native-ESM jest (`importMode:
 * 'import'` + Node vm modules). CommonJS output matches TypeScript's
 * `module: "commonjs"` + `esModuleInterop` emit. ESM-only dependencies
 * (`.mjs` / `node_modules`) are downleveled to the runner's module format with
 * the Angular passes skipped.
 */
export function createTransformer(
  transformerOptions: OxcAngularJestOptions = {},
): JestSyncTransformer {
  const derived = transformerOptions.tsconfig
    ? deriveTransformOptions(transformerOptions.tsconfig)
    : {};
  const importMode = transformerOptions.importMode || derived.importMode || 'auto';
  const esm = transformerOptions.esm ?? derived.esm ?? false;
  // Resolve whether output modules are ESM (controls the stringified module form).
  const esmOutput =
    importMode === 'import' ? true : importMode === 'require' ? false : esm;
  // `null`/`''` disables stringification.
  const stringifyPattern =
    transformerOptions.stringifyContentPathRegex === undefined
      ? '\\.(html|svg)$'
      : transformerOptions.stringifyContentPathRegex;
  const stringifyRe = stringifyPattern ? new RegExp(stringifyPattern) : null;

  return {
    canInstrument: true,
    // Include the native transform version so jest's transform cache is
    // invalidated when the binding (and thus its output) changes.
    getCacheKey(sourceText, sourcePath, options) {
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
        importMode,
        esm,
        coverage: collectCoverage || Boolean(transformerOptions.coverage),
        // ESM deps are plain JS: only downlevel the module format, skip the
        // Angular JIT passes (they'd be no-ops but cost a traversal).
        ...(isDep ? { jitTransforms: false } : {}),
        ...transformerOptions.transform,
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
