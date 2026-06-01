import * as fs from 'node:fs';
import { transform, type TransformOptions } from '@oxc-angular-testing/transform';
import { deriveTransformOptions } from '@oxc-angular-testing/transform/tsconfig';
import type { Plugin, ResolvedConfig } from 'vite';

const TS_RE = /\.[cm]?tsx?(\?|$)/;
const HTML_RE = /\.html(\?|$)/;
const NODE_MODULES_RE = /\/node_modules\//;

export interface OxcAngularOptions {
  /**
   * Fold istanbul coverage instrumentation into the same AST pass. When omitted,
   * it is auto-enabled if vitest is run with the `istanbul` coverage provider.
   */
  coverage?: boolean;
  /** Path to a tsconfig to derive target / module / decorator flags from. */
  tsconfig?: string;
  /** Override individual transform options forwarded to the Rust transform. */
  transform?: Partial<TransformOptions>;
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
 * Defaults to ESM `import` mode (the natural fit for Vitest). Pass `{ tsconfig }`
 * to derive `target` / decorator flags from the project tsconfig (explicit
 * `transform` overrides win). Coverage is auto-enabled when vitest runs with the
 * `istanbul` provider; pass `{ coverage: true | false }` to force it.
 */
export default function oxcAngular(options: OxcAngularOptions = {}): Plugin {
  const derived = options.tsconfig ? deriveTransformOptions(options.tsconfig) : {};
  let autoCoverage = false;

  return {
    name: '@oxc-angular-testing/vitest',
    enforce: 'pre',

    configResolved(config: ResolvedConfig) {
      const cov = (config as ResolvedConfig & VitestAwareConfig).test?.coverage;
      // Our transform emits istanbul `__coverage__`; only auto-enable for the
      // istanbul provider (v8 uses runtime coverage and needs no instrumentation).
      autoCoverage = cov?.enabled === true && cov.provider === 'istanbul';
    },

    // Load component `templateUrl` HTML as a default-exported string module.
    load: {
      filter: { id: HTML_RE },
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
          importMode: 'import',
          esm: true,
          ...derived,
          coverage: options.coverage ?? autoCoverage,
          ...options.transform,
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
