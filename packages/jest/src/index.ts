import * as crypto from 'node:crypto';
import * as fs from 'node:fs';
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

// Verify, once per worker process, that the istanbul this project actually uses
// recognizes our emitted coverage schema marker. jest's `generateEmptyCoverage`
// calls `istanbul-lib-instrument`'s `readInitialCoverage` to report never-imported
// `collectCoverageFrom` files as 0%; if istanbul ever changes its schema, that call
// silently SKIPS our output (it doesn't throw) and those files vanish from the
// report. Round-trip a probe through the consumer's own istanbul and fail loudly if
// it isn't recognized.
let coverageSchemaChecked = false;
function verifyCoverageSchemaOnce(): void {
  if (coverageSchemaChecked) return;
  coverageSchemaChecked = true;

  // Resolve the istanbul-lib-instrument jest itself uses (relative to jest /
  // @jest/reporters), falling back to this package's own resolution.
  let readInitialCoverage: ((code: string) => unknown) | undefined;
  let version = 'unknown';
  const bases: string[] = [];
  for (const pkg of ['jest', '@jest/reporters']) {
    try {
      bases.push(path.dirname(require.resolve(`${pkg}/package.json`)));
    } catch {
      /* not installed under this name; try the next */
    }
  }
  bases.push(__dirname);
  for (const base of bases) {
    try {
      const ili = require(
        require.resolve('istanbul-lib-instrument', { paths: [base] }),
      ) as { readInitialCoverage?: (code: string) => unknown };
      if (typeof ili.readInitialCoverage === 'function') {
        readInitialCoverage = ili.readInitialCoverage.bind(ili);
        try {
          version = (
            require(
              require.resolve('istanbul-lib-instrument/package.json', { paths: [base] }),
            ) as { version: string }
          ).version;
        } catch {
          /* keep 'unknown' */
        }
        break;
      }
    } catch {
      /* try the next base */
    }
  }

  if (!readInitialCoverage) {
    console.warn(
      '@oxc-angular-testing/jest: could not resolve istanbul-lib-instrument to verify the ' +
        'coverage schema marker; skipping the check. (Never-imported-file coverage relies on it.)',
    );
    return;
  }
  const probe = transform('const __oxc_probe__ = 1;\n', '__oxc_coverage_probe__.ts', {
    coverage: true,
    module: 'commonjs',
    jitTransforms: false,
  });
  if (!readInitialCoverage(probe.code)) {
    throw new Error(
      `@oxc-angular-testing/jest: your installed istanbul-lib-instrument@${version} does not ` +
        "recognize this transform's coverage schema marker, so never-imported files would be " +
        'silently dropped from coverage. This usually means istanbul changed its coverage schema — ' +
        'please file an issue against @oxc-angular-testing.',
    );
  }
}

// A local rebuild of the native binding (or of this plugin) doesn't bump the
// package version, so `transformVersion` alone serves STALE cached transforms
// during development — a rebuilt `.node` with the old cache looks like "the fix
// didn't work". Fold in the binding files' size+mtime and this file's own mtime
// (statted once per worker); in a published install these are fixed, so cache
// behavior there is unchanged.
const bindingStamp = (() => {
  const parts: string[] = [];
  try {
    parts.push(String(fs.statSync(__filename).mtimeMs));
  } catch {
    /* keep what we have */
  }
  try {
    const dir = path.dirname(require.resolve('@oxc-angular-testing/transform/package.json'));
    // Sorted: readdir order is filesystem-dependent, and the stamp must be
    // deterministic for stable cache keys.
    for (const f of fs.readdirSync(dir).sort()) {
      if (f.endsWith('.node')) {
        const s = fs.statSync(path.join(dir, f));
        parts.push(`${f}:${s.size}:${s.mtimeMs}`);
      }
    }
  } catch {
    /* prebuilt binding packages — version alone suffices */
  }
  return parts.join(',');
})();

// Runtime helpers (`_decorate`, `_async_to_generator`, …) are emitted as
// `require("@oxc-project/runtime/helpers/<name>")` — a dependency of
// `@oxc-angular-testing/transform`, NOT of the consumer. Node (and jest-resolve)
// resolve that specifier from the *transformed file's* directory, so under a
// package manager with isolated node_modules (pnpm) the bare specifier is
// unresolvable from application code and every decorated file fails to load.
// Resolve the package once, relative to the transform package itself, and emit
// helpers by absolute path instead — correct under every layout (hoisted npm,
// isolated pnpm, linked local checkouts). CommonJS output only: `require()`
// resolves extensionless absolute paths, while an ESM `import` would not (and
// a Windows drive path is not a valid ESM specifier), so ESM output keeps the
// bare specifier. Resolving a known helper file (rather than the package root)
// honors the exports map, which rewrites `./helpers/*` into `./src/helpers/*`;
// stripping `/helpers/decorate.js` leaves the prefix oxc appends
// `helpers/<name>` to. `null` (resolution failed) falls back to the bare
// specifier — the pre-existing behavior.
let cjsHelperModuleName: string | null | undefined;
function resolveCjsHelperModuleName(): string | undefined {
  if (cjsHelperModuleName === undefined) {
    try {
      const transformDir = path.dirname(
        require.resolve('@oxc-angular-testing/transform/package.json'),
      );
      const decorate = require.resolve('@oxc-project/runtime/helpers/decorate', {
        paths: [transformDir],
      });
      cjsHelperModuleName = path.dirname(path.dirname(decorate));
    } catch {
      cjsHelperModuleName = null;
    }
  }
  return cjsHelperModuleName ?? undefined;
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
  // `module` is set from the tsconfig/preset; `coverage` is controlled by the
  // dedicated top-level `coverage` option + jest's instrument signal (see process()).
  transform?: Partial<Omit<TransformOptions, 'module' | 'coverage'>>;
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
 * runner's module format with the Angular/TS passes skipped — i.e. a `.mjs` file,
 * or a `.js` file under `node_modules` (e.g. `@angular/core`, which ships only ESM
 * and must be converted to CommonJS for classic CJS jest). Additional regex sources
 * can be supplied via `processEsmModules`.
 *
 * Mirrors jest-preset-angular's `processWithEsbuild` fast path (jest-preset-angular
 * 16: `**​/*.mjs` globs, plus `filePath.includes('node_modules') && endsWith('.js')`),
 * but uses our own oxc ESM→CJS transform instead of esbuild. The `node_modules`
 * substring is intentionally unanchored, matching jest-preset-angular (also covers
 * Windows `\node_modules\` and pnpm/Yarn-PnP layouts).
 */
export function isEsmDependency(
  sourcePath: string,
  processEsmModules?: string[],
): boolean {
  if (
    sourcePath.endsWith('.mjs') ||
    (sourcePath.includes('node_modules') && sourcePath.endsWith('.js'))
  ) {
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
  // Memoized per (expanded tsconfig path + rootDir/cwd), NOT once for the instance:
  // jest creates one transformer per project today (so a single entry), but keying on
  // the resolved config means a future shared/multi-rootDir instance can't silently
  // serve the first file's derived options to a different project.
  type Resolved = {
    derived: DerivedTransformOptions;
    moduleKind: 'commonjs' | 'esm';
    esmOutput: boolean;
  };
  const resolvedByKey = new Map<string, Resolved>();
  const resolve = (config?: { rootDir?: string; cwd?: string }): Resolved => {
    const rootDir = config?.rootDir;
    const base = rootDir ?? config?.cwd;
    const tsconfigPath = transformerOptions.tsconfig
      ? expandRootDir(transformerOptions.tsconfig, rootDir)
      : '';
    const key = `${tsconfigPath}\0${base ?? ''}`;
    const cached = resolvedByKey.get(key);
    if (cached) return cached;
    let derived: DerivedTransformOptions = {};
    if (transformerOptions.tsconfig) {
      derived = deriveTransformOptions(tsconfigPath, base);
    }
    const moduleKind = transformerOptions.module ?? derived.module ?? 'commonjs';
    // ESM output controls the stringified content module form (export vs module.exports).
    const result: Resolved = { derived, moduleKind, esmOutput: moduleKind === 'esm' };
    resolvedByKey.set(key, result);
    return result;
  };

  // The helper module name actually baked into the output: an explicit
  // `transform.helperModuleName` override wins, else the resolved absolute
  // path for CJS output only (ESM keeps the bare specifier). Shared by
  // process() and getCacheKey() so the cache is keyed on exactly what emits.
  const effectiveHelperModuleName = (moduleKind: 'commonjs' | 'esm'): string | undefined =>
    transformerOptions.transform?.helperModuleName ??
    (moduleKind === 'commonjs' ? resolveCjsHelperModuleName() : undefined);

  return {
    canInstrument: true,
    // Include the native transform version so jest's transform cache is
    // invalidated when the binding (and thus its output) changes.
    getCacheKey(sourceText, sourcePath, options) {
      const { derived, moduleKind } = resolve(options?.config);
      return crypto
        .createHash('sha1')
        .update(transformVersion)
        .update('\0')
        .update(bindingStamp)
        .update('\0')
        .update(JSON.stringify(transformerOptions))
        .update('\0')
        .update(JSON.stringify(derived))
        .update('\0')
        // The EFFECTIVE helper name is baked into the output — a moved/
        // reinstalled node_modules must not serve stale absolute paths from
        // jest's cache. ESM output (bare specifier) hashes nothing extra.
        .update(effectiveHelperModuleName(moduleKind) ?? '')
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
        // Effective helper name (override, else absolute path for CJS — see
        // effectiveHelperModuleName); placed after the transform spread because
        // it already folds the override in.
        helperModuleName: effectiveHelperModuleName(moduleKind),
        module: moduleKind,
        // Coverage precedence (identical in the vitest plugin): an explicit
        // top-level `coverage` option wins (true OR false); otherwise derive from
        // the runner — here jest's per-file `instrument` signal. `coverage` is not
        // settable via `transform` (excluded from its type), so there is no second,
        // inconsistent knob.
        coverage: transformerOptions.coverage ?? collectCoverage,
        // Hoist `jest.mock()` above imports for the user's test code (jest
        // requires this). Deps are library code with no jest.mock + skip JIT;
        // force hoisting off for them so a `transform` override can't leak in.
        ...(isDep
          ? { jitTransforms: false, hoistMock: undefined }
          : { hoistMock: 'jest' as const }),
      };
      if (opts.coverage) verifyCoverageSchemaOnce();
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
