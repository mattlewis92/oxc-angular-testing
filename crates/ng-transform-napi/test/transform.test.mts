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

test('require mode inlines template and strips styles', () => {
  const out = transform(COMPONENT, 'foo.component.ts', { importMode: 'require' });
  assert.equal(out.errors.length, 0, out.errors.join('\n'));
  assert.match(out.code, /template: require\("\.\/foo\.component\.html"\)/);
  assert.ok(!out.code.includes('styleUrls'));
  assert.ok(!out.code.includes('templateUrl'));
});

test('import mode hoists a top-level import', () => {
  const out = transform(COMPONENT, 'foo.component.ts', { importMode: 'import' });
  assert.match(out.code, /import __NG_CLI_RESOURCE__0 from "\.\/foo\.component\.html"/);
  assert.match(out.code, /template: __NG_CLI_RESOURCE__0/);
});

test('coverage instrumentation in a single pass', () => {
  const out = transform('function add(a, b) { return a + b; }', 'add.js', { coverage: true });
  assert.ok(out.code.includes('__coverage__'), out.code);
  assert.ok(out.coverageMap, 'coverageMap present');
  assert.match(out.coverageMap, /fnMap/);
});
