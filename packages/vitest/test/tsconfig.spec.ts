import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { deriveTransformOptions } from '@oxc-angular-testing/transform/tsconfig';

describe('@oxc-angular-testing/transform/tsconfig', () => {
  it('derives target / module / decorator flags from a tsconfig', () => {
    const tsconfig = fileURLToPath(new URL('./fixtures/tsconfig.fixture.json', import.meta.url));
    const opts = deriveTransformOptions(tsconfig);
    expect(opts.target).toBe('es2015');
    expect(opts.module).toBe('commonjs'); // tsconfig module: commonjs
    expect(opts.experimentalDecorators).toBe(true);
    expect(opts.emitDecoratorMetadata).toBe(true);
    // useDefineForClassFields defaults to false for target < ES2022.
    expect(opts.useDefineForClassFields).toBe(false);
  });
});
