import { Service, TOKEN, helper } from './fixtures/esm-dep';

describe('@oxc-angular-testing/jest — ESM dependency downleveling (CommonJS)', () => {
  it('downlevels an ESM dependency (imports, named exports, class, re-export) to CJS', () => {
    // esm-dep.js + esm-helper.js are authored in ESM; the transform converted
    // them to CommonJS so classic CJS jest could require them.
    expect(TOKEN).toBe('tok');
    expect(new Service().name()).toBe('svc:helped');
    expect(helper()).toBe('helped'); // re-exported binding
  });
});
