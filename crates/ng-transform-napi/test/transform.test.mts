import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { test } from 'node:test';
import { transform } from '../index.js';
import { scriptTargetToString } from '../dist/tsconfig.js';

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

test('every SCRIPT_TARGET string round-trips through oxc (target-vocabulary canary)', () => {
  // tsconfig.ts maps every `ts.ScriptTarget` to one of these strings, and each MUST
  // be accepted by oxc's `EnvOptions::from_target`. A mismatch (e.g. the old ES3/ES5
  // → 'es5', which oxc rejects) used to be swallowed into no-downleveling — a silent
  // miscompile. The transform now pushes a diagnostic on a bad target, so this canary
  // turns a JS↔oxc vocabulary drift (an oxc rename, or a new mapping oxc doesn't know)
  // into a named CI failure. Keys are the numeric `ts.ScriptTarget` enum values.
  const scriptTargets = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 99];
  for (const k of scriptTargets) {
    const target = scriptTargetToString(k);
    const out = transform('export const x = 1;', 'x.ts', { target, jitTransforms: false });
    assert.equal(
      out.errors.length,
      0,
      `target "${target}" (ScriptTarget ${k}) was rejected by oxc: ${out.errors.join(', ')}`,
    );
  }
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

test('helperModuleName rewrites the runtime-helper import prefix', () => {
  // Legacy decorator lowering imports `_decorate` from `@oxc-project/runtime` —
  // a dependency of this package, not of the consumer. `helperModuleName` lets
  // a runner emit the require via a resolved path instead (pnpm's isolated
  // node_modules can't reach our dependency from the transformed file).
  const src = 'function dec(t: any) { return t; }\n@dec\nexport class C {}\n';
  const bare = transform(src, 'c.ts', { module: 'commonjs', jitTransforms: false });
  assert.equal(bare.errors.length, 0, bare.errors.join('\n'));
  assert.match(bare.code, /require\("@oxc-project\/runtime\/helpers\/decorate"\)/);
  const abs = transform(src, 'c.ts', {
    module: 'commonjs',
    jitTransforms: false,
    helperModuleName: '/abs/rt/src',
  });
  assert.equal(abs.errors.length, 0, abs.errors.join('\n'));
  assert.match(abs.code, /require\("\/abs\/rt\/src\/helpers\/decorate"\)/);
  assert.ok(!abs.code.includes('"@oxc-project/runtime'), 'no bare specifier remains');
});

test('BigInt literals pass through untouched below es2020 (tsc transpile parity)', () => {
  // tsc in transpile-only mode never errors on `1n` at a lower target (TS2737 is
  // a checker diagnostic); oxc's env flag would hard-error. We match tsc.
  const out = transform('export const big = 123n + BigInt(4);\n', 'big.ts', {
    module: 'commonjs',
    target: 'es2019',
    jitTransforms: false,
  });
  assert.equal(out.errors.length, 0, out.errors.join('\n'));
  assert.match(out.code, /123n/);
});

test('exported function declarations hoist exports assignment above requires (tsc parity)', () => {
  // tsc emits `exports.f = f;` directly after the `__esModule` marker — before
  // any `require` — because function declarations hoist. A circular importer
  // that re-enters this module mid-evaluation must already see the function
  // (ts-jest/tsc behavior). Classes/consts can't hoist and keep the late
  // assignment + `void 0` header.
  const src = [
    "import { o } from './o';",
    'export function f() { return o(); }',
    'export default function d() { return 1; }',
    'function g() { return 2; }',
    'export { g };',
    'export class C {}',
    '',
  ].join('\n');
  const out = transform(src, 'm.ts', { module: 'commonjs', jitTransforms: false });
  assert.equal(out.errors.length, 0, out.errors.join('\n'));
  const code = out.code;
  const reqIdx = code.indexOf('require("./o")');
  assert.ok(reqIdx > 0, code);
  for (const assign of ['exports.f = f', 'exports.default = d', 'exports.g = g']) {
    const i = code.indexOf(assign);
    assert.ok(i >= 0 && i < reqIdx, `${assign} must be hoisted above the require:\n${code}`);
  }
  // The class keeps tsc's shape: in the `void 0` header, assigned late.
  assert.match(code, /exports\.C = void 0/);
  assert.ok(code.indexOf('exports.C = C') > reqIdx, code);
  // Hoisted function exports are NOT in the `void 0` header.
  const header = code.slice(0, reqIdx);
  assert.ok(!/exports\.(f|g|default) = void 0/.test(header), header);
});

test('strictNullChecks=false serializes decorator metadata like tsc SNC-off', () => {
  // tsc's serializer drops null/undefined union constituents when
  // strictNullChecks is OFF (`string | null` → String); strict mode yields
  // Object. Compare both modes against tsc's documented behavior.
  const src = [
    'function dec(): any { return () => {}; }',
    '@dec()',
    'export class Svc {',
    '  @dec() a: string | null = null;',
    '  @dec() c: string | number | null = null;',
    '  constructor(@dec() x: Date | null, @dec() y?: number) {}',
    '  @dec() m(z: string | undefined): string | null { return z ?? null; }',
    '}',
  ].join('\n');
  const opts = {
    module: 'commonjs',
    jitTransforms: false,
    experimentalDecorators: true,
    emitDecoratorMetadata: true,
    target: 'es2019',
  };
  const off = transform(src, 's.ts', { ...opts, strictNullChecks: false });
  assert.equal(off.errors.length, 0, off.errors.join('\n'));
  assert.match(off.code, /"design:type", String/); // a: string|null → String
  assert.match(off.code, /"design:returntype", String/); // string|null → String
  assert.match(off.code, /"design:paramtypes", \[String\]/); // [string|undefined] → [String]
  // string|number|null → still a two-type union → Object.
  assert.match(off.code, /"design:type", Object/);
  // Date|null → Date reference (guarded or bare), NOT Object.
  const ctor = off.code.match(/"design:paramtypes", \[([^\]]*Date[^\]]*)\]/);
  assert.ok(ctor, `ctor paramtypes serialize Date:\n${off.code}`);

  // Default (strict) keeps the current oxc behavior: unions with null → Object.
  const on = transform(src, 's.ts', opts);
  assert.ok(!/"design:type", String/.test(on.code), on.code);
});

test('jest.mock hoists BELOW the @jest/globals import (no TDZ after CJS rewrite)', () => {
  const src = [
    "import { jest } from '@jest/globals';",
    "import { helper } from './helper';",
    "jest.mock('./x', () => ({}));",
    'helper();',
  ].join('\n');
  const out = transform(src, 't.ts', { module: 'commonjs', jitTransforms: false, hoistMock: 'jest' });
  assert.equal(out.errors.length, 0, out.errors.join('\n'));
  const code = out.code;
  const globalsIdx = code.indexOf('require("@jest/globals")');
  const mockIdx = code.indexOf('.jest.mock("./x"');
  const helperIdx = code.indexOf('require("./helper")');
  assert.ok(globalsIdx >= 0 && mockIdx >= 0 && helperIdx >= 0, code);
  assert.ok(globalsIdx < mockIdx, `@jest/globals require above the hoisted mock:\n${code}`);
  assert.ok(mockIdx < helperIdx, `hoisted mock above other requires:\n${code}`);
});

test('enum members initialized from another same-file string enum fold like tsc', () => {
  // tsc constant-folds `B.X = A.Y` to the string literal, emitting the
  // string-enum form; an unfolded reference makes oxc emit the numeric-enum
  // form with a bogus reverse mapping — Object.values(B) then contains phantom
  // member NAMES, breaking `it.each(Object.values(E))`-style consumers.
  const src = [
    'export enum BaseEnum {',
    "  RED = 'red',",
    "  BLUE = 'blue',",
    '}',
    'export enum ExtendedEnum {',
    "  NONE = 'none',",
    '  RED = BaseEnum.RED,',
    '  BLUE = BaseEnum.BLUE,',
    '}',
  ].join('\n');
  const out = transform(src, 'e.ts', { module: 'commonjs', jitTransforms: false, target: 'es2019' });
  assert.equal(out.errors.length, 0, out.errors.join('\n'));
  const mod = { exports: {} as Record<string, Record<string, string>> };
  new Function('exports', 'module', out.code)(mod.exports, mod);
  assert.deepEqual(mod.exports.ExtendedEnum, {
    NONE: 'none',
    RED: 'red',
    BLUE: 'blue',
  });
  // Cross-file references stay as runtime expressions (per-file transpile
  // cannot fold them; matches tsc transpileModule).
  const cross = transform(
    "import { A } from './a';\nexport enum B { X = A.Y }\n",
    'b.ts',
    { module: 'commonjs', jitTransforms: false, target: 'es2019' },
  );
  assert.match(cross.code, /A\.Y/);
});

test('side-effect import + named import of the same module keeps the require var', () => {
  // `import './m'; import { x } from './m';` — per-source dedup must not let
  // the bare side-effect require swallow the `const m_1 = require("./m")`
  // declaration the rewritten references (`m_1.x`) depend on.
  const out = transform(
    "import './m';\nimport { x } from './m';\nexport const y = x;\n",
    's.ts',
    { module: 'commonjs', jitTransforms: false },
  );
  assert.equal(out.errors.length, 0, out.errors.join('\n'));
  const sandbox = { exports: {} as Record<string, unknown> };
  new Function('require', 'exports', 'module', out.code)(
    () => ({ x: 42 }),
    sandbox.exports,
    sandbox,
  );
  assert.equal(sandbox.exports.y, 42);
  assert.equal(out.code.match(/require\("\.\/m"\)/g)?.length, 1, out.code);
});
