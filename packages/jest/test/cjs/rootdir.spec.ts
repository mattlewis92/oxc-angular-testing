import * as path from 'node:path';
import { createTransformer } from '../../dist/index.js';

// R10 regression: jest does NOT expand `<rootDir>` inside a transformer's option
// object, so the plugin must expand it itself against `options.config.rootDir`
// (mirroring ts-jest's `resolvePath`). Without that, a `{ tsconfig:
// '<rootDir>/tsconfig.json' }` reads nothing → no `target` derived → oxc defaults
// to esnext (async left native, etc.). Asserted at the emit level (no zone needed).
const ASYNC = 'export async function f() { return 1; }\n';
const fixtures = path.join(__dirname, 'fixtures'); // contains tsconfig.rootdir.json (target es2016)

describe('jest plugin: <rootDir> tsconfig resolution (R10)', () => {
  it('expands <rootDir> against options.config.rootDir → derives target es2016 → downlevels async', () => {
    const t = createTransformer({ tsconfig: '<rootDir>/tsconfig.rootdir.json' });
    const code = t.process(ASYNC, path.join(fixtures, 'x.ts'), { config: { rootDir: fixtures } }).code;
    expect(code).toMatch(/function\* *\(/); // async lowered to a generator (es2016 derived)
    expect(code).not.toMatch(/\basync function\b/);
  });

  it('without a resolvable rootDir, the unexpanded <rootDir> tsconfig is a hard error', () => {
    // No options.config → `<rootDir>` can't expand → the literal path is unreadable.
    // That is a misconfiguration we refuse to skip: deriveTransformOptions throws
    // rather than silently falling back to defaults (which would miscompile).
    const t = createTransformer({ tsconfig: '<rootDir>/tsconfig.rootdir.json' });
    expect(() => t.process(ASYNC, path.join(fixtures, 'x.ts'), {})).toThrow(
      /could not read tsconfig|<rootDir>/,
    );
  });
});
