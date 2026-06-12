# oxc-angular-testing

Fast Angular transforms for unit tests, implemented in Rust on
[oxc](https://github.com/oxc-project/oxc) and exposed to Node via napi.

A drop-in-spirit reimplementation of the source transforms
[`jest-preset-angular`](https://github.com/thymikee/jest-preset-angular) applies
to Angular code under test — component resource inlining, Angular decorator
downleveling, and the JIT signal-initializer-API decorators — plus optional
**istanbul-compatible coverage instrumentation folded into the same AST pass**
(one parse, one codegen) via
[`oxc-coverage-instrument`](https://github.com/fallow-rs/oxc-coverage-instrument).

## Packages

| Package | What it is |
| --- | --- |
| `@oxc-angular-testing/transform` | napi bindings to the Rust transform. Per-platform binaries ship as `@oxc-angular-testing/binding-*` optional deps. |
| `@oxc-angular-testing/jest` | Jest transformer wiring up the transform. |
| `@oxc-angular-testing/vitest` | Vitest/Vite plugin wiring up the transform. |

No Rust crates are published — only the npm packages.

## Pipeline

```
parse → semantic → Angular passes → oxc TS/decorator lowering → [coverage] → codegen
        (one oxc_allocator::Allocator, one parse, one codegen)
```

The coverage pass runs `oxc_coverage_instrument`'s visitor on the *same*
arena-allocated AST the Angular passes mutate, via a small vendored patch
(`vendor/`, see `vendor/VENDORED.md`) that exposes a post-parse
`instrument_program` entry. All oxc crates — ours and the vendored coverage
crate — are pinned to **oxc 0.126** so the AST types are shared.

## Usage

### Vitest

```ts
// vitest.config.ts
import { defineConfig } from 'vitest/config';
import oxcAngular from '@oxc-angular-testing/vitest';

export default defineConfig({
  plugins: [oxcAngular()],
});
```

Coverage auto-enables when vitest runs with the `istanbul` provider
(`test.coverage.provider: 'istanbul'`); pass `oxcAngular({ coverage: true | false })`
to force it.

#### Component styles (`keepStyles`)

By default component styles are **stripped** (`styles`, `styleUrl`, `styleUrls`
removed) — matching jest-preset-angular, and right for node/jsdom unit tests
where layout doesn't exist anyway. Under vitest **browser mode**, real-layout
tests need styles applied, so the plugin keeps them there: when `keepStyles` is
not set, it is decided per Vite environment — styles are kept for the `client`
environment (browser mode) and stripped for `ssr` (node/jsdom projects). Pass
`oxcAngular({ keepStyles: true | false })` to force one behavior everywhere.

When styles are kept, the transform does **not** compile any CSS itself —
that's vite's job (an explicit non-goal: no sass in the transform).
`styleUrl`/`styleUrls` entries are rewritten to hoisted default imports with
the `inline` query the plugin passes (e.g. `import __oxc_ng_style_0__ from
'./a.scss?inline'`), which vite's CSS pipeline (with your sass/less/postcss
config) compiles to a CSS string, and the decorator property is replaced with
`styles: [__oxc_ng_style_0__]` — which Angular JIT accepts as-is. Inline
`styles` are preserved and merged ahead of the URL-derived entries, matching
Angular's own resolution order.

### Jest (ESM)

```js
// jest.config.mjs — CommonJS (classic jest)
import { createCjsPreset } from '@oxc-angular-testing/jest/presets';
export default { ...createCjsPreset({ tsconfig: './tsconfig.spec.json' }) };
```

```js
// jest.config.mjs — native ESM (run with NODE_OPTIONS=--experimental-vm-modules)
import { createEsmPreset } from '@oxc-angular-testing/jest/presets';
export default { ...createEsmPreset({ tsconfig: './tsconfig.spec.json' }) };
```

The presets set `transform`, `transformIgnorePatterns` and `moduleFileExtensions`
for you. You can also wire the transformer manually (`'^.+\\.(ts|js)$':
['@oxc-angular-testing/jest', { module: 'commonjs' }]`).

### ESM-only dependencies

Like jest-preset-angular's esbuild fast path, the jest plugin downlevels ESM
dependencies (`.mjs` / `node_modules`, e.g. `@angular/core`) to the runner's
module format with the Angular passes skipped — using our own oxc ESM→CJS
transform, not esbuild. The CJS preset sets
`transformIgnorePatterns: ['node_modules/(?!.*\\.mjs$)']` so `.mjs` files in
`node_modules` (including `@angular/*`, which ships `.mjs`) reach the transformer
instead of being ignored.

> **How `.mjs` works under classic CJS jest:** jest only routes modules to its
> ESM loader when run with `--experimental-vm-modules`. The **CJS preset runs
> *without* that flag**, so jest transforms every matched file — including
> `.mjs` (e.g. `@angular/core`) — to CommonJS and `require()`s it. This works on
> all supported Node versions (no Node ≥ 24.9 needed); it's how jest-preset-angular
> handles Angular too. Run the CJS and ESM presets as **separate** jest
> invocations, since the vm-modules flag is process-global (`jest --selectProjects`
> or two configs). The ESM preset *does* use the flag and loads `.mjs` natively.

### Direct

```js
import { transform } from '@oxc-angular-testing/transform';

const { code, map, coverageMap, errors } = transform(source, 'foo.component.ts', {
  module: 'commonjs', // 'commonjs' | 'esm' — drives templateUrl require/import + ESM→CJS
  coverage: false,
});
```

`@oxc-angular-testing/transform` depends on `@oxc-project/runtime` for the
decorator/class helpers the lowering emits.

## Status

| Transform | Status |
| --- | --- |
| `templateUrl` → `template` (`require`/`import` per `module`) | ✅ `resources.rs` |
| `styleUrls` / `styleUrl` / `styles` / `moduleId` stripping | ✅ `resources.rs` |
| `keepStyles`: style URLs → `?inline` imports, merged `styles: [...]` (vitest browser mode) | ✅ `resources.rs` |
| Constructor/decorator downleveling (`ctorParameters`/`propDecorators`) | ✅ `jit_transform.rs` |
| Signal initializer-API decorators (`input()`/`output()`/`model()`/queries) | ✅ `jit_transform.rs` |
| TS → JS + legacy decorator lowering, ES `target` downleveling | ✅ via `oxc_transformer` |
| ESM → CommonJS (matches `tsc` `module:commonjs` + `esModuleInterop`) | ✅ `esm_to_cjs.rs` |
| Dynamic `import()` → `require` (matches `tsc`) | ✅ `esm_to_cjs.rs` |
| `jest.mock()` hoisting (babel-plugin-jest-hoist) | ✅ `jest_hoist.rs` (jest plugin) |
| JSX/TSX for mixed Angular + React (automatic/classic, from tsconfig `jsx`) | ✅ via `oxc_transformer` |
| istanbul coverage in the same AST pass | ✅ vendored `instrument_program` |
| Options derived from tsconfig (target / module / decorators / `useDefineForClassFields` / `jsx`) | ✅ `transform/src/tsconfig.ts` |
| ESM-only dependency downleveling for jest (esbuild-fast-path equivalent) | ✅ jest plugin + `presets.ts` |
| jest (ESM **and** CommonJS) + vitest plugins, real component integration tests | ✅ |

Every row is covered by tests: `cargo test --workspace` (resources, JIT, ESM→CJS,
lowering, coverage) plus the jest/vitest integration suites.

### Notes

- **CommonJS** output matches TypeScript's `module: "commonjs"` + `esModuleInterop`
  emit (`__importDefault`/`__importStar`/`__exportStar` interop, `(0, m_1.x)()`
  call wrapping, `exports.x = …`, `__esModule`). Re-exports use assignment rather
  than `Object.defineProperty` getters (runnable-equivalent for static re-exports).
- **`target`** maps to oxc's `EnvOptions::from_target`; only syntax newer than the
  target is downleveled. **`lower: false`** is a test-only switch to inspect the
  pre-lowering TypeScript AST.
- **`useDefineForClassFields: false`** (the default, Angular's setting) emits class
  fields as plain assignments (oxc `set_public_class_fields` +
  `remove_class_fields_without_initializer`).
- **`keepStyles`** (default `false`) keeps component styles instead of stripping
  them, for tests that exercise real layout (vitest browser mode). Style URLs
  become default imports (ESM) or `require(...)` calls (CommonJS) — the
  bundler's CSS pipeline owns compilation (no sass in the transform, by
  design). **`keepStylesQuery`** (default unset — URLs emitted verbatim) names
  a query parameter to append to each rewritten URL: the vitest plugin
  hard-codes `'inline'`, so vite yields the CSS text, which Angular JIT accepts
  in `styles: [...]` (URLs that already carry a query get `&inline`). The
  hoisted identifiers (`__oxc_ng_style_N__`) are deterministic and dodge user
  bindings. Note the CommonJS form is emitted for completeness: plain jest has
  no CSS pipeline to resolve `require('./a.scss?inline')`, so the jest plugin
  does not expose the option — `keepStyles` is a vitest (vite) feature.
- **Content stringification** — files matching `stringifyContentPathRegex`
  (default `\.(html|svg)$`) are returned as a string module (their raw content)
  rather than compiled, so component `templateUrl` HTML and inline SVG imports
  work. The jest presets route `.html`/`.svg` through this; the vitest plugin's
  `load` hook does the same. Mirrors jest-preset-angular's option of the same name.
- **Coverage baseline** — instrumentation runs on the **source** (before any
  transform), so the coverage map is target-independent and matches
  `istanbul-lib-instrument` byte-for-byte on the same source (enforced by the
  differential test in `transform/test/coverage-differential.test.mts`). This is a
  deliberate, more-truthful baseline. `jest-preset-angular` (ts-jest) instead
  instruments the **compiled CommonJS output**, so its `%` reads slightly *higher*
  on the same code: it counts two always-covered structural nodes that don't exist
  in the source — the synthesized field-init **constructor** (a function) and the
  CJS **export plumbing** (`exports.X = …`, statements). Both are always hit, and
  since the base fraction is < 100%, including them raises the percentage. Same
  covered/uncovered *lines*, different denominator. **Migrating from
  jest-preset-angular:** re-baseline your `coverageThreshold`s against this
  transform's output — run once with `--coverage` and set the thresholds from the
  reported numbers (they reflect the real, executable surface), e.g. derive them
  from `coverage/coverage-summary.json`.

## Development

```sh
pnpm install
cargo test --workspace   # Rust transforms + coverage
pnpm build               # native binding (napi) + TypeScript packages (tsc → dist/)
pnpm typecheck           # tsc --noEmit across all packages
pnpm test                # binding + jest (esm+cjs) + vitest
```

The npm package sources are TypeScript (`packages/*/src`, `crates/ng-transform-napi/src`)
compiled to `dist/` by `tsc`; the napi binding's `index.js`/`index.d.ts` are
generated by `napi build`. Run `pnpm build` before `pnpm test`/`pnpm typecheck`
(jest loads the built transformer; vitest runs against source via its
`vitest.config.ts`). Test fixtures keep their deliberate module formats
(`.mjs`/`.cjs`/`.js`) and are not converted.

Toolchain: pnpm 11, Node ≥ 20.19 / 22.12, Rust stable (oxc pinned to 0.126).

### Releasing

The package.json `version` fields are placeholders (`0.0.0`) — the release
workflow (`.github/workflows/release-npm.yml`) is the source of truth for the
published version. Trigger it either by pushing a `v*` tag (e.g. `v1.2.3`) or via
**workflow_dispatch** with an explicit `version` input. The workflow stamps that
version across all three packages with `scripts/set-version.mjs`; `napi
prepublish` then propagates it to the `@oxc-angular-testing/binding-*` platform
packages + the main package's `optionalDependencies`, and `pnpm publish` rewrites
jest/vitest's `workspace:*` dependency on `@oxc-angular-testing/transform` to that
exact version — so all packages publish in lockstep. Publishing is idempotent: a
package is skipped only if that exact version is already on npm, so a re-run after
a partial failure completes the rest.
