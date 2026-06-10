import assert from 'node:assert/strict';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { test } from 'node:test';
import ts from 'typescript';
import { deriveTransformOptions } from '../dist/tsconfig.js';

// deriveTransformOptions memoizes per resolved tsconfig path at module scope,
// so every test writes its fixtures to a fresh directory — reusing a path would
// hand later tests a cached result and silently skip the code under test.
function fixtureDir(): string {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'oxc-ng-tsconfig-'));
}

function writeConfig(dir: string, name: string, config: unknown): string {
  const file = path.join(dir, name);
  fs.writeFileSync(file, JSON.stringify(config));
  return file;
}

test('derives merged options through an extends chain', () => {
  const dir = fixtureDir();
  writeConfig(dir, 'base.json', {
    compilerOptions: {
      target: 'es2017',
      module: 'commonjs',
      experimentalDecorators: true,
      emitDecoratorMetadata: true,
    },
  });
  const child = writeConfig(dir, 'tsconfig.json', {
    extends: './base.json',
    compilerOptions: { target: 'es2022' },
  });
  assert.deepEqual(deriveTransformOptions(child), {
    target: 'es2022',
    module: 'commonjs',
    experimentalDecorators: true,
    emitDecoratorMetadata: true,
    // TS defaults useDefineForClassFields to true at effective target >= ES2022.
    useDefineForClassFields: true,
  });
});

test('maps module kinds: commonjs stays commonjs, everything else is esm', () => {
  const dir = fixtureDir();
  const cjs = writeConfig(dir, 'cjs.json', { compilerOptions: { module: 'commonjs' } });
  const esnext = writeConfig(dir, 'esnext.json', { compilerOptions: { module: 'esnext' } });
  const nodenext = writeConfig(dir, 'nodenext.json', {
    compilerOptions: { module: 'nodenext', moduleResolution: 'nodenext' },
  });
  const none = writeConfig(dir, 'none.json', { compilerOptions: {} });
  assert.equal(deriveTransformOptions(cjs).module, 'commonjs');
  assert.equal(deriveTransformOptions(esnext).module, 'esm');
  assert.equal(deriveTransformOptions(nodenext).module, 'esm');
  assert.equal(deriveTransformOptions(none).module, undefined);
});

test('maps targets, clamping ES5 to the oxc es2015 floor', () => {
  const dir = fixtureDir();
  const es5 = writeConfig(dir, 'es5.json', { compilerOptions: { target: 'es5' } });
  const es2016 = writeConfig(dir, 'es2016.json', { compilerOptions: { target: 'es2016' } });
  const esnext = writeConfig(dir, 'esnext.json', { compilerOptions: { target: 'esnext' } });
  assert.equal(deriveTransformOptions(es5).target, 'es2015');
  assert.equal(deriveTransformOptions(es2016).target, 'es2016');
  assert.equal(deriveTransformOptions(esnext).target, 'esnext');
});

test('useDefineForClassFields: explicit value wins, default follows effective target', () => {
  const dir = fixtureDir();
  const old = writeConfig(dir, 'old.json', { compilerOptions: { target: 'es2021' } });
  const modern = writeConfig(dir, 'modern.json', { compilerOptions: { target: 'es2022' } });
  const explicitOff = writeConfig(dir, 'off.json', {
    compilerOptions: { target: 'es2022', useDefineForClassFields: false },
  });
  // No explicit target: module nodenext raises the EFFECTIVE target to ES2022+.
  const viaModule = writeConfig(dir, 'via-module.json', {
    compilerOptions: { module: 'nodenext', moduleResolution: 'nodenext' },
  });
  assert.equal(deriveTransformOptions(old).useDefineForClassFields, false);
  assert.equal(deriveTransformOptions(modern).useDefineForClassFields, true);
  assert.equal(deriveTransformOptions(explicitOff).useDefineForClassFields, false);
  assert.equal(deriveTransformOptions(viaModule).useDefineForClassFields, true);
});

test('maps jsx variants', () => {
  const dir = fixtureDir();
  const classic = writeConfig(dir, 'classic.json', {
    compilerOptions: { jsx: 'react', jsxFactory: 'h', jsxFragmentFactory: 'Frag' },
  });
  const automatic = writeConfig(dir, 'automatic.json', {
    compilerOptions: { jsx: 'react-jsx', jsxImportSource: 'preact' },
  });
  const dev = writeConfig(dir, 'dev.json', { compilerOptions: { jsx: 'react-jsxdev' } });
  const preserve = writeConfig(dir, 'preserve.json', { compilerOptions: { jsx: 'preserve' } });

  const c = deriveTransformOptions(classic);
  assert.equal(c.jsx, 'classic');
  assert.equal(c.jsxFactory, 'h');
  assert.equal(c.jsxFragmentFactory, 'Frag');

  const a = deriveTransformOptions(automatic);
  assert.equal(a.jsx, 'automatic');
  assert.equal(a.jsxDevelopment, false);
  assert.equal(a.jsxImportSource, 'preact');

  assert.equal(deriveTransformOptions(dev).jsxDevelopment, true);
  assert.equal(deriveTransformOptions(preserve).jsx, 'automatic');
});

test('a tsconfig whose include matches no files does not throw (TS18002/TS18003 filtered)', () => {
  const dir = fixtureDir();
  const noMatch = writeConfig(dir, 'tsconfig.json', {
    compilerOptions: { target: 'es2020' },
    include: ['no-such-dir/**/*.ts'],
  });
  // Also the `files: []` shape, which TS reports as TS18002 rather than TS18003.
  const emptyFiles = writeConfig(dir, 'empty-files.json', {
    compilerOptions: { target: 'es2020' },
    files: [],
  });
  assert.equal(deriveTransformOptions(noMatch).target, 'es2020');
  assert.equal(deriveTransformOptions(emptyFiles).target, 'es2020');
});

test('genuinely broken tsconfigs still throw', () => {
  const dir = fixtureDir();
  const badExtends = writeConfig(dir, 'bad-extends.json', {
    extends: './does-not-exist.json',
    compilerOptions: {},
  });
  const badOption = writeConfig(dir, 'bad-option.json', {
    compilerOptions: { target: 'es9999' },
  });
  const malformed = path.join(dir, 'malformed.json');
  fs.writeFileSync(malformed, '{ not json');
  assert.throws(() => deriveTransformOptions(badExtends), /has errors/);
  assert.throws(() => deriveTransformOptions(badOption), /has errors/);
  assert.throws(() => deriveTransformOptions(malformed), /could not read tsconfig/);
});

test('memoizes per resolved path: second call returns the same object', () => {
  const dir = fixtureDir();
  const config = writeConfig(dir, 'tsconfig.json', {
    compilerOptions: { target: 'es2022', module: 'commonjs' },
  });
  const first = deriveTransformOptions(config);
  assert.equal(deriveTransformOptions(config), first);
  // A relative path resolving to the same file hits the same cache entry.
  assert.equal(deriveTransformOptions('tsconfig.json', dir), first);
});

test('never enumerates project files, even with include globs and an extends chain', () => {
  const dir = fixtureDir();
  // A real tree the include glob WOULD match if anything walked it.
  fs.mkdirSync(path.join(dir, 'src', 'nested'), { recursive: true });
  fs.writeFileSync(path.join(dir, 'src', 'a.ts'), 'export {};');
  fs.writeFileSync(path.join(dir, 'src', 'nested', 'b.spec.ts'), 'export {};');
  writeConfig(dir, 'base.json', {
    compilerOptions: { target: 'es2021' },
    include: ['**/*.ts'],
  });
  const child = writeConfig(dir, 'tsconfig.json', {
    extends: './base.json',
    compilerOptions: { experimentalDecorators: true },
    include: ['**/*.spec.ts', '**/*.d.ts'],
  });

  // deriveTransformOptions passes `ts.sys` to parseJsonConfigFileContent, and
  // glob expansion goes through host.readDirectory — so a poisoned
  // ts.sys.readDirectory proves no directory walk happens.
  const original = ts.sys.readDirectory;
  ts.sys.readDirectory = () => {
    throw new Error('deriveTransformOptions must not enumerate project files');
  };
  try {
    const opts = deriveTransformOptions(child);
    assert.equal(opts.target, 'es2021');
    assert.equal(opts.experimentalDecorators, true);
  } finally {
    ts.sys.readDirectory = original;
  }
});
