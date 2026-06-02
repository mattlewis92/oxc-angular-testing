import assert from 'node:assert/strict';
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

test('downleveled async inlines the helper and returns the global Promise (R10)', () => {
  // At es2016 async IS downleveled — but the helper is inlined (not imported from
  // a separate-realm runtime module), so the result Promise is the module's
  // global. Under zone.js that's the zone-patched `Promise`, so `instanceof` /
  // `expect.any(Promise)` hold.
  const out = transform('export async function f() { return 1; }', 'f.ts', {
    module: 'commonjs',
    jitTransforms: false,
    target: 'es2016',
  }).code;
  assert.match(out, /_asyncToGenerator\(function\*/, 'async downleveled to a generator');
  assert.doesNotMatch(
    out,
    /require\("@oxc-project\/runtime\/helpers\/asyncToGenerator"\)/,
    'helper inlined, not imported from a separate realm',
  );
  assert.match(out, /var _asyncToGenerator = \(function/, 'inline helper definition present');

  // The result Promise is the module's (here reassigned) global Promise.
  class ZoneAwarePromise extends Promise {}
  const realPromise = globalThis.Promise;
  (globalThis as { Promise: PromiseConstructor }).Promise =
    ZoneAwarePromise as unknown as PromiseConstructor;
  try {
    const mod: { exports: { f(): unknown } } = { exports: { f: () => undefined } };
    // The inlined-helper async output needs no `require`; pass a stub.
    const noRequire = () => {
      throw new Error('unexpected require');
    };
    new Function('exports', 'module', 'require', out)(mod.exports, mod, noRequire);
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
