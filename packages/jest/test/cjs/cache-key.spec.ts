import { createTransformer } from '../../dist/index.js';

// The cache key must be a pure function of the transform's effective inputs:
// deterministic across transformer instances (the binding stamp reads the
// filesystem — sorted, so directory order can't destabilize it), sensitive to
// everything that changes the output (source, instrument, helper override),
// and equal under ESM for option sets whose only difference (the resolved CJS
// helper path) never reaches ESM output.
const SRC = 'export const x: number = 1;\n';

describe('@oxc-angular-testing/jest — cache key', () => {
  it('is deterministic across fresh transformer instances', () => {
    const a = createTransformer({ module: 'commonjs' });
    const b = createTransformer({ module: 'commonjs' });
    expect(a.getCacheKey(SRC, '/p/x.ts', {})).toBe(b.getCacheKey(SRC, '/p/x.ts', {}));
  });

  it('changes with source, instrument, and a helper override', () => {
    const t = createTransformer({ module: 'commonjs' });
    const base = t.getCacheKey(SRC, '/p/x.ts', {});
    expect(t.getCacheKey(`${SRC};`, '/p/x.ts', {})).not.toBe(base);
    expect(t.getCacheKey(SRC, '/p/x.ts', { instrument: true })).not.toBe(base);
    const o = createTransformer({
      module: 'commonjs',
      transform: { helperModuleName: '/custom/rt' },
    });
    expect(o.getCacheKey(SRC, '/p/x.ts', {})).not.toBe(base);
  });

  it('keys CJS and ESM output differently', () => {
    const cjs = createTransformer({ module: 'commonjs' });
    const esm = createTransformer({ module: 'esm' });
    expect(cjs.getCacheKey(SRC, '/p/x.ts', {})).not.toBe(esm.getCacheKey(SRC, '/p/x.ts', {}));
  });
});
