import * as fs from 'node:fs';
import * as path from 'node:path';
import { transform, type TransformOptions } from '@oxc-angular-testing/transform';
import {
  deriveTransformOptions,
  type DerivedTransformOptions,
} from '@oxc-angular-testing/transform/tsconfig';
import type { Plugin, ResolvedConfig } from 'vite';

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
   * `module` is intentionally excluded: Vitest always runs native ESM, so this
   * plugin only ever emits ESM.
   */
  transform?: Partial<Omit<TransformOptions, 'module'>>;
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
  // is known: a relative or `<rootDir>`-prefixed tsconfig path is resolved
  // against it. Deriving at plugin-construction time (before the root is known)
  // would mis-resolve such paths, read nothing, and silently drop `target` etc.
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
        const root = config.root;
        const tsconfigPath = options.tsconfig.startsWith('<rootDir>')
          ? path.join(root, options.tsconfig.slice('<rootDir>'.length))
          : options.tsconfig;
        derived = deriveTransformOptions(tsconfigPath, root);
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
          coverage: options.coverage ?? autoCoverage,
          ...options.transform,
          // Vitest runs native ESM: force `esm` last so neither the
          // tsconfig-derived options nor an explicit override can select CJS.
          module: 'esm',
        };
        const out = transform(code, id.split('?')[0]!, opts);
        if (out.errors && out.errors.length > 0) {
          this.error(out.errors.join('\n'));
        }
        return { code: out.code, map: out.map ? JSON.parse(out.map) : null };
      },
    },
  };
}
