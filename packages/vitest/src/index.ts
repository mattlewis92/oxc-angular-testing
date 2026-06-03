import * as fs from 'node:fs';
import { createRequire } from 'node:module';
import * as path from 'node:path';
import { transform, type TransformOptions } from '@oxc-angular-testing/transform';
import {
  deriveTransformOptions,
  type DerivedTransformOptions,
} from '@oxc-angular-testing/transform/tsconfig';
import type { Plugin, ResolvedConfig } from 'vite';

const require_ = createRequire(import.meta.url);

// Verify, once per process, that the istanbul the istanbul coverage provider uses
// recognizes our emitted coverage schema marker (`_coverageSchema`). istanbul's
// `readInitialCoverage` silently SKIPS (does not throw) an unrecognized marker, so
// a schema change would drop never-imported files from coverage with no error.
// Round-trip a probe through the consumer's own istanbul and fail loudly otherwise.
let coverageSchemaChecked = false;
function verifyCoverageSchemaOnce(): void {
  if (coverageSchemaChecked) return;
  coverageSchemaChecked = true;

  let readInitialCoverage: ((code: string) => unknown) | undefined;
  let version = 'unknown';
  const bases: string[] = [];
  for (const pkg of ['@vitest/coverage-istanbul', 'vitest']) {
    try {
      bases.push(path.dirname(require_.resolve(`${pkg}/package.json`)));
    } catch {
      /* not installed; try the next */
    }
  }
  for (const base of bases) {
    try {
      const ili = require_(
        require_.resolve('istanbul-lib-instrument', { paths: [base] }),
      ) as { readInitialCoverage?: (code: string) => unknown };
      if (typeof ili.readInitialCoverage === 'function') {
        readInitialCoverage = ili.readInitialCoverage.bind(ili);
        try {
          version = (
            require_(
              require_.resolve('istanbul-lib-instrument/package.json', { paths: [base] }),
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
      '@oxc-angular-testing/vitest: could not resolve istanbul-lib-instrument to verify the ' +
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
      `@oxc-angular-testing/vitest: your installed istanbul-lib-instrument@${version} does not ` +
        "recognize this transform's coverage schema marker, so never-imported files would be " +
        'silently dropped from coverage. This usually means istanbul changed its coverage schema — ' +
        'please file an issue against @oxc-angular-testing.',
    );
  }
}

const TS_RE = /\.[cm]?tsx?(\?|$)/;
const NODE_MODULES_RE = /\/node_modules\//;
const DEFAULT_STRINGIFY_RE = /\.(html|svg)(\?|$)/;

export interface OxcAngularOptions {
  /**
   * Fold istanbul coverage instrumentation into the same AST pass. When omitted,
   * it is auto-enabled if vitest is run with the `istanbul` coverage provider.
   */
  coverage?: boolean;
  /** Path to a tsconfig to derive target / decorator flags from. */
  tsconfig?: string;
  /**
   * Files whose id matches are loaded as a default-exported string module (raw
   * content) instead of compiled — component `templateUrl` HTML and inline SVG.
   * Default matches `.html` and `.svg`.
   */
  stringifyContentPathRegex?: RegExp;
  /**
   * Override individual transform options forwarded to the Rust transform.
   * `module` is intentionally excluded (Vitest always runs native ESM, so this
   * plugin only ever emits ESM), as is `coverage` (controlled by the dedicated
   * top-level `coverage` option + the auto-detected istanbul provider).
   */
  transform?: Partial<Omit<TransformOptions, 'module' | 'coverage'>>;
}

// Vitest augments Vite's resolved config with a `test` field; type the slice we
// read without taking a hard dependency on vitest's types.
interface VitestAwareConfig {
  test?: { coverage?: { enabled?: boolean; provider?: string } };
}

/**
 * Vitest/Vite plugin that runs the oxc Angular test transform on TypeScript
 * files (resource inlining, decorator downleveling, signal initializer APIs)
 * and loads component `templateUrl` HTML as a string module.
 *
 * Always emits ESM (`import` mode) — Vitest runs native ESM, so there is no CJS
 * path here (the module kind is never taken from tsconfig). Pass `{ tsconfig }`
 * to derive `target` / decorator flags from the project tsconfig (explicit
 * `transform` overrides win). Coverage is auto-enabled when vitest runs with the
 * `istanbul` provider; pass `{ coverage: true | false }` to force it.
 */
export default function oxcAngular(options: OxcAngularOptions = {}): Plugin {
  // tsconfig derivation is deferred to `configResolved`, where the project root
  // is known, so a relative `tsconfig` path resolves against the Vite/Vitest root
  // rather than `process.cwd()`. (Vitest has no `<rootDir>` placeholder — that is
  // jest-only — so a relative or absolute path is all we handle here.) Deriving at
  // plugin-construction time (before the root is known) would mis-resolve a
  // relative path, read nothing, and silently drop `target`/decorator flags.
  let derived: DerivedTransformOptions = {};
  const stringifyRe = options.stringifyContentPathRegex ?? DEFAULT_STRINGIFY_RE;
  let autoCoverage = false;

  return {
    name: '@oxc-angular-testing/vitest',
    enforce: 'pre',

    configResolved(config: ResolvedConfig) {
      const cov = (config as ResolvedConfig & VitestAwareConfig).test?.coverage;
      // Our transform emits istanbul `__coverage__`; only auto-enable for the
      // istanbul provider (v8 uses runtime coverage and needs no instrumentation).
      autoCoverage = cov?.enabled === true && cov.provider === 'istanbul';

      if (options.tsconfig) {
        // Resolve a relative tsconfig against the Vitest root (not cwd).
        derived = deriveTransformOptions(options.tsconfig, config.root);
      }
    },

    // Load component `templateUrl` HTML / inline SVG as a string module.
    load: {
      filter: { id: stringifyRe },
      handler(id: string) {
        const file = id.split('?')[0]!;
        const src = fs.readFileSync(file, 'utf8');
        return { code: `export default ${JSON.stringify(src)};`, map: null };
      },
    },

    // Transform TypeScript (excluding node_modules) via the oxc Angular transform.
    transform: {
      filter: { id: { include: TS_RE, exclude: NODE_MODULES_RE } },
      handler(code: string, id: string) {
        const opts: TransformOptions = {
          ...derived,
          ...options.transform,
          // Coverage precedence (identical in the jest plugin): an explicit
          // top-level `coverage` option wins (true OR false); otherwise derive from
          // the runner — here the auto-detected istanbul provider. Computed AFTER the
          // `transform` spread, and `coverage` is excluded from `transform`'s type, so
          // there is no second, inconsistent knob.
          coverage: options.coverage ?? autoCoverage,
          // Vitest runs native ESM: force `esm` last so neither the
          // tsconfig-derived options nor an explicit override can select CJS.
          module: 'esm',
        };
        if (opts.coverage) verifyCoverageSchemaOnce();
        const out = transform(code, id.split('?')[0]!, opts);
        if (out.errors && out.errors.length > 0) {
          this.error(out.errors.join('\n'));
        }
        return { code: out.code, map: out.map ? JSON.parse(out.map) : null };
      },
    },
  };
}
