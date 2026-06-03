import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { test } from 'node:test';
import { transform } from '../index.js';

const COMPONENT = `import { Component } from '@angular/core';
@Component({
  selector: 'app-foo',
  templateUrl: './foo.component.html',
  styleUrls: ['./foo.component.css'],
})
export class FooComponent {}
`;

test('commonjs mode inlines template via require and strips styles', () => {
  const out = transform(COMPONENT, 'foo.component.ts', { module: 'commonjs' });
  assert.equal(out.errors.length, 0, out.errors.join('\n'));
  assert.match(out.code, /template: require\("\.\/foo\.component\.html"\)/);
  assert.ok(!out.code.includes('styleUrls'));
  assert.ok(!out.code.includes('templateUrl'));
});

test('esm mode hoists a top-level import', () => {
  const out = transform(COMPONENT, 'foo.component.ts', { module: 'esm' });
  assert.match(out.code, /import __NG_CLI_RESOURCE__0 from "\.\/foo\.component\.html"/);
  assert.match(out.code, /template: __NG_CLI_RESOURCE__0/);
});

test('coverage instrumentation in a single pass', () => {
  const out = transform('function add(a, b) { return a + b; }', 'add.js', { coverage: true });
  assert.ok(out.code.includes('__coverage__'), out.code);
  assert.ok(out.coverageMap, 'coverageMap present');
  assert.match(out.coverageMap, /fnMap/);
});

test('coverage does not count synthesized functions (no phantom constructor)', () => {
  // A class field with `useDefineForClassFields: false` (Angular default) makes
  // oxc synthesize a constructor to host the init. Istanbul must not count that
  // generated function, else coverage differs from a babel/jest-preset setup.
  const out = transform('export class C { x = 1; m() { return this.x; } }', 'c.ts', {
    module: 'commonjs',
    coverage: true,
    jitTransforms: false,
  });
  const fns = Object.values(JSON.parse(out.coverageMap).fnMap).map((f: any) => f.name);
  assert.deepEqual(fns, ['m'], `only the real method, no synthesized constructor: ${JSON.stringify(fns)}`);
});

test('async method downleveled at es2016 is counted once at its real location', () => {
  // Repro: async→generator downleveling wraps `return 42` in a synthetic
  // generator. The generator must not be counted as an extra function, and the
  // real `load` function/loc must point at the source (not the synthetic 1:0).
  const src =
    'export class Calc {\n  add(a, b) { return a + b; }\n  async load() { return 42; }\n}\n';
  const out = transform(src, 'calc.ts', {
    module: 'commonjs',
    coverage: true,
    target: 'es2016',
    jitTransforms: false,
  });
  const cov = JSON.parse(out.coverageMap);
  const fns = Object.values(cov.fnMap) as any[];
  assert.deepEqual(
    fns.map((f) => f.name).sort(),
    ['add', 'load'],
    `exactly add + load, no synthetic generator: ${JSON.stringify(fns.map((f) => f.name))}`,
  );
  assert.ok(
    fns.every((f) => f.decl.start.line > 1 && f.loc.start.line > 1),
    `no function attributed to the synthetic line 1: ${JSON.stringify(fns.map((f) => [f.name, f.loc.start.line]))}`,
  );
});

test('downleveled async uses the runtime helper and returns the global Promise (R10)', () => {
  // At es2016 async is downleveled to oxc's `asyncToGenerator` runtime helper
  // (imported from @oxc-project/runtime — a SEPARATE module, not inlined). Its
  // bare, late-bound `new Promise` resolves to the realm-global `Promise` at call
  // time, so under zone.js the result is the zone-patched `Promise` and
  // `instanceof` / `expect.any(Promise)` hold. (The native, non-downleveled path
  // at esnext cannot be made zone-aware — it uses the V8 %Promise% intrinsic.)
  const out = transform('export async function f() { return 1; }', 'f.ts', {
    module: 'commonjs',
    jitTransforms: false,
    target: 'es2016',
  }).code;
  assert.doesNotMatch(out, /\basync function\b/, 'async downleveled, not left native');
  assert.match(out, /function\* *\(/, 'downleveled to a generator');
  assert.match(
    out,
    /require\("@oxc-project\/runtime\/helpers\/asyncToGenerator"\)/,
    'helper imported from @oxc-project/runtime',
  );

  // The separate-module helper's `new Promise` is late-bound, so it resolves to
  // the module's (here reassigned) global Promise — proving it is zone-safe.
  const nodeRequire = createRequire(import.meta.url);
  class ZoneAwarePromise extends Promise {}
  const realPromise = globalThis.Promise;
  (globalThis as { Promise: PromiseConstructor }).Promise =
    ZoneAwarePromise as unknown as PromiseConstructor;
  try {
    const mod: { exports: { f(): unknown } } = { exports: { f: () => undefined } };
    new Function('exports', 'module', 'require', out)(mod.exports, mod, nodeRequire);
    assert.ok(mod.exports.f() instanceof ZoneAwarePromise, 'returns the global (zone-patched) Promise');
  } finally {
    (globalThis as { Promise: PromiseConstructor }).Promise = realPromise;
  }

  // Other syntax still downlevels at the same target.
  const nullishOut = transform('export const x = a ?? b;', 'g.ts', {
    module: 'commonjs',
    jitTransforms: false,
    target: 'es2016',
  }).code;
  assert.ok(!nullishOut.includes('??'), 'nullish coalescing still downleveled at es2016');
});

test('coverage keeps the `this` receiver on optional-chaining method calls (R22)', () => {
  // Coverage instrumentation wrapped the optional-chain CALLEE in a counter call
  // (`cov_oc(obj?.method, id)?.()`), evaluating it to a detached function → the
  // method ran with `this === undefined`. The receiver must survive. Run the
  // emitted (instrumented, es2016-downleveled) code and assert the method that
  // reads `this.value` returns it.
  const out = transform(
    'export function makeObj() { return { value: 42, getValue() { return this.value; } }; }\n' +
      'export function callMethod(obj) { return obj?.getValue?.(); }\n',
    'm.ts',
    { module: 'commonjs', target: 'es2016', coverage: true, jitTransforms: false },
  ).code;
  // The instrumented callee stays a member access (receiver-preserving), not a
  // bare `cov_*_oc(...)?.()`.
  assert.doesNotMatch(out, /_oc\([^)]*\)\?\.\(\)/, 'optional method-call callee must not be detached');
  const mod: { exports: { makeObj(): unknown; callMethod(o: unknown): unknown } } = {
    exports: { makeObj: () => undefined, callMethod: () => undefined },
  };
  const noRequire = () => {
    throw new Error('unexpected require');
  };
  new Function('exports', 'module', 'require', out)(mod.exports, mod, noRequire);
  assert.equal(mod.exports.callMethod(mod.exports.makeObj()), 42, 'this.value via obj?.getValue?.()');
});

test('namespace import members are spy-friendly: configurable + settable (R12)', () => {
  // `import * as ns from 'cjs-dep'` → __importStar/__createBinding getter shim.
  // It must be configurable (so jest.spyOn can redefine) and settable.
  const out = transform("import * as ns from './m';\nns.x();\n", 'm.ts', {
    module: 'commonjs',
    jitTransforms: false,
  });
  assert.match(out.code, /configurable: true/, 'namespace getter must be configurable');
  assert.match(out.code, /set: function\(v\) \{ m\[k\] = v; \}/, 'namespace member must be settable');
});

test('branch coverage shape is independent of the ES target (source-level)', () => {
  // Instrumenting before downleveling means `?.` is always 2 optional-chain
  // branches — not 1 cond-expr after an es2015 rewrite. This is what keeps
  // coverage stable regardless of the project's tsconfig target.
  const src = 'export function f(a) { return a?.b?.c; }\n';
  const branchTypes = (target: string) =>
    Object.values(
      JSON.parse(
        transform(src, 'f.ts', { module: 'commonjs', coverage: true, jitTransforms: false, target })
          .coverageMap,
      ).branchMap,
    ).map((b: any) => b.type);
  const esnext = branchTypes('esnext');
  const es2015 = branchTypes('es2015');
  assert.deepEqual(esnext, ['optional-chain', 'optional-chain'], 'two optional-chain branches');
  assert.deepEqual(es2015, esnext, `branch shape must not change with target: ${JSON.stringify({ es2015, esnext })}`);
});

test('every statement counter is emitted — no dead counters (exported fn-init declarators)', () => {
  // Regression: `export const f = () => …` is an ExportNamedDeclaration whose
  // span starts at `export`, but the per-declarator statement counter is hoisted
  // to the inner VariableDeclaration's start. If those offsets aren't reconciled
  // the `++s[0]` is dropped, so the declaration statement is never counted and
  // coverage under-reports (1/2 instead of 2/2). Assert the map has no statement
  // id without a matching increment in the emitted code, for the forms that hit
  // this path (arrow/function/class init, with and without `export`).
  for (const src of [
    'export const f = (x) => x * 2;',
    'export const f = (x) => { return x * 2; };',
    'export const C = class { m(x) { return x * 2; } };',
    'const f = (x) => x * 2;\nmodule.exports.f = f;',
  ]) {
    const out = transform(src, 'f.ts', { module: 'commonjs', coverage: true, jitTransforms: false });
    const stmtIds = Object.keys(JSON.parse(out.coverageMap).statementMap);
    for (const id of stmtIds) {
      assert.match(
        out.code,
        new RegExp(`\\.s\\[${id}\\]`),
        `statement ${id} has no emitted counter (dead counter → under-counts): ${src}`,
      );
    }
  }
});
