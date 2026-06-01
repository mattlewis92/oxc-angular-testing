import { createTransformer } from '../../dist/index.js';

// When jest runs with `--coverage`, it passes `{ instrument: true }` to the
// transformer's `process()` / `getCacheKey()` (see @jest/transform
// `ReducedTransformOptions.instrument`). Because we report `canInstrument: true`,
// jest trusts our output as already-instrumented and will NOT re-instrument it
// with Babel — so `process()` must emit istanbul instrumentation in that case,
// and the cache key must depend on `instrument` (otherwise a previously-cached
// non-instrumented result would be served when coverage is requested).
describe('@oxc-angular-testing/jest — coverage instrumentation', () => {
  const transformer = createTransformer({ importMode: 'require', esm: false });
  const src = [
    'export function classify(n: number) {',
    '  if (n > 0) {',
    '    return "pos";',
    '  }',
    '  return "nonpos";',
    '}',
    '',
  ].join('\n');

  it('reports canInstrument so jest trusts our instrumentation', () => {
    expect(transformer.canInstrument).toBe(true);
  });

  it('emits istanbul instrumentation when jest requests coverage', () => {
    const out = transformer.process(src, '/proj/classify.ts', { instrument: true });
    expect(out.code).toContain('__coverage__'); // global coverage store
    expect(out.code).toMatch(/cov_\w+/); // per-file coverage object
    expect(out.code).toContain('statementMap');
    expect(out.code).toContain('fnMap');
    expect(out.code).toContain('branchMap'); // the if/else is tracked
  });

  it('does not instrument when coverage is off', () => {
    const out = transformer.process(src, '/proj/classify.ts', { instrument: false });
    expect(out.code).not.toContain('__coverage__');
    // still a working CJS transform
    expect(out.code).toContain('exports.classify');
  });

  it('varies the cache key by instrument (no stale non-instrumented cache hit)', () => {
    const withCov = transformer.getCacheKey(src, '/proj/classify.ts', { instrument: true });
    const noCov = transformer.getCacheKey(src, '/proj/classify.ts', { instrument: false });
    expect(withCov).not.toBe(noCov);
  });
});
