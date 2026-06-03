import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { test } from 'node:test';
// Register the JIT compiler so executing a decorated class compiles it (sets ɵcmp /
// ɵprov), exactly as a real test environment does.
import '@angular/compiler';
import { Injector, reflectComponentType } from '@angular/core';
import { transform } from '../index.js';

// These tests validate our emitted metadata against the REAL @angular/core JIT
// compiler — not just its textual shape (which the other suites assert). They are the
// canary that catches a divergence between what we synthesize and what Angular's
// runtime actually accepts (signal-input registration, the delegate-ctor reflection
// regex for inherited DI). No TestBed / zone needed: reflectComponentType and
// Injector.create drive JIT compilation directly.
const require_ = createRequire(import.meta.url);

// Transform `src` to CJS and execute it with a require that resolves the real
// @angular/core, returning the module's exports (with live ɵcmp/ɵprov getters).
function loadModule(src: string, filename: string): Record<string, unknown> {
  const out = transform(src, filename, { module: 'commonjs', target: 'es2016' });
  assert.equal(out.errors.length, 0, out.errors.join('\n'));
  const mod: { exports: Record<string, unknown> } = { exports: {} };
  new Function('exports', 'module', 'require', out.code)(mod.exports, mod, require_);
  return mod.exports;
}

test('signal input() is registered as a real Angular component input (JIT)', () => {
  const exports = loadModule(
    `import { Component, input } from '@angular/core';
@Component({ selector: 'app-x', template: '' })
export class XComponent {
  value = input(0);
  label = input.required();
}
`,
    'x.component.ts',
  );
  const mirror = reflectComponentType(exports.XComponent as never);
  assert.ok(mirror, 'real JIT compiled the component (reflectComponentType returned a mirror)');
  const names = mirror.inputs.map((i: { propName: string }) => i.propName).sort();
  assert.deepEqual(
    names,
    ['label', 'value'],
    `both signal inputs must be registered by the real compiler: ${JSON.stringify(names)}`,
  );
});

test('derived @Injectable inherits DI params through the rewritten delegate ctor (JIT)', () => {
  // Child has a class field, so oxc synthesizes the delegating constructor that our
  // delegate_ctor pass rewrites to `super(...arguments)` — the exact shape Angular's
  // INHERITED_CLASS_WITH_DELEGATE_CTOR reflection regex requires to inherit Base's DI
  // params. If that rewrite (or the regex coupling) ever breaks, this fails.
  const exports = loadModule(
    `import { Injectable } from '@angular/core';
@Injectable() export class Dep { value = 42; }
@Injectable() export class Base { constructor(public dep: Dep) {} }
@Injectable() export class Child extends Base { extra = 1; }
`,
    'svc.ts',
  );
  const injector = Injector.create({
    providers: [exports.Dep, exports.Base, exports.Child] as never,
  });
  const child = injector.get(exports.Child as never) as { dep: { value: number }; extra: number };
  assert.ok(child.dep, 'inherited constructor dependency was injected (delegate-ctor inheritance)');
  assert.equal(child.dep.value, 42);
  assert.equal(child.extra, 1);
});
