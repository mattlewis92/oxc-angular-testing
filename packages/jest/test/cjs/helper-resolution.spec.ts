import * as fs from 'node:fs';
import * as path from 'node:path';
import { createTransformer } from '../../dist/index.js';

// Runtime helpers (`_decorate`, …) are a dependency of
// `@oxc-angular-testing/transform`, not of the consumer; under a package manager
// with isolated node_modules (pnpm) the bare
// `require("@oxc-project/runtime/helpers/…")` emitted into a consumer's file is
// unresolvable from that file's location and every decorated file fails to load.
// CommonJS output therefore emits the helper require via an absolute path
// resolved from the transform package itself.
const DECORATED = [
  'function dec(target: any) { return target; }',
  '@dec',
  'export class C {}',
  '',
].join('\n');

describe('@oxc-angular-testing/jest — CJS runtime-helper resolution', () => {
  it('emits helper requires as absolute paths that exist on disk', () => {
    const t = createTransformer({ module: 'commonjs', transform: { jitTransforms: false } });
    const code = t.process(DECORATED, '/consumer/app/c.ts', {}).code;
    const m = code.match(/require\("([^"]*helpers\/decorate)"\)/);
    expect(m).not.toBeNull();
    const spec = m![1];
    expect(path.isAbsolute(spec)).toBe(true);
    // Node resolves the extensionless require target to `…/decorate.js`.
    expect(fs.existsSync(`${spec}.js`)).toBe(true);
  });

  it('an explicit transform.helperModuleName override wins', () => {
    const t = createTransformer({
      module: 'commonjs',
      transform: { jitTransforms: false, helperModuleName: '@custom/rt' },
    });
    const code = t.process(DECORATED, '/consumer/app/c.ts', {}).code;
    expect(code).toContain('require("@custom/rt/helpers/decorate")');
  });

  it('ESM output keeps the bare specifier (an extensionless absolute path is not importable)', () => {
    const t = createTransformer({ module: 'esm', transform: { jitTransforms: false } });
    const code = t.process(DECORATED, '/consumer/app/c.ts', {}).code;
    expect(code).toMatch(/from "@oxc-project\/runtime\/helpers\//);
  });
});
