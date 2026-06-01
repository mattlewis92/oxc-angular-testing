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
['@oxc-angular-testing/jest', { importMode: 'require' }]`).

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
  importMode: 'auto', // 'auto' | 'require' | 'import'
  esm: false,
  coverage: false,
});
```

`@oxc-angular-testing/transform` depends on `@oxc-project/runtime` for the
decorator/class helpers the lowering emits.

## Status

| Transform | Status |
| --- | --- |
| `templateUrl` → `template` (`require`/`import` per `importMode`) | ✅ `resources.rs` |
| `styleUrls` / `styleUrl` / `styles` / `moduleId` stripping | ✅ `resources.rs` |
| Constructor/decorator downleveling (`ctorParameters`/`propDecorators`) | ✅ `jit_transform.rs` |
| Signal initializer-API decorators (`input()`/`output()`/`model()`/queries) | ✅ `jit_transform.rs` |
| TS → JS + legacy decorator lowering, ES `target` downleveling | ✅ via `oxc_transformer` |
| ESM → CommonJS (matches `tsc` `module:commonjs` + `esModuleInterop`) | ✅ `esm_to_cjs.rs` |
| istanbul coverage in the same AST pass | ✅ vendored `instrument_program` |
| Options derived from tsconfig (target / module / decorators / `useDefineForClassFields`) | ✅ `transform/src/tsconfig.ts` |
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
- **Content stringification** — files matching `stringifyContentPathRegex`
  (default `\.(html|svg)$`) are returned as a string module (their raw content)
  rather than compiled, so component `templateUrl` HTML and inline SVG imports
  work. The jest presets route `.html`/`.svg` through this; the vitest plugin's
  `load` hook does the same. Mirrors jest-preset-angular's option of the same name.

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
