// A dependency authored in ESM (import / named export / class / re-export).
// jest loads `.js` here via the CJS loader (the package is `type: commonjs`),
// so the transform must downlevel the ESM syntax to CommonJS for it to run.
// (Real `.mjs` / `"type":"module"` deps are loaded as ESM by jest regardless of
// transform — those need the ESM preset or Node >= 24.9; see the README.)
import { helper } from './esm-helper.js';

export const TOKEN = 'tok';

export class Service {
  name() {
    return `svc:${helper()}`;
  }
}

export { helper } from './esm-helper.js';
