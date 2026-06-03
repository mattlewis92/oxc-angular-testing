import { isEsmDependency } from '../../dist/index.js';

// Mirrors jest-preset-angular 16's fast-path classification: `.mjs` anywhere, or a
// `.js` file under node_modules (unanchored substring). A node_modules `.ts`/other
// is NOT a fast-path ESM dep.
describe('@oxc-angular-testing/jest — isEsmDependency', () => {
  it('treats any .mjs as an ESM dep', () => {
    expect(isEsmDependency('/proj/src/foo.mjs')).toBe(true);
    expect(isEsmDependency('/proj/node_modules/x/index.mjs')).toBe(true);
  });

  it('treats a .js file under node_modules as an ESM dep', () => {
    expect(isEsmDependency('/proj/node_modules/@angular/core/index.js')).toBe(true);
    // unanchored substring also covers Windows + pnpm layouts (matches jest-preset-angular)
    expect(isEsmDependency('C:\\proj\\node_modules\\x\\index.js')).toBe(true);
    expect(isEsmDependency('/proj/node_modules/.pnpm/x@1/node_modules/x/y.js')).toBe(true);
  });

  it('does NOT treat a non-.js node_modules file as an ESM dep', () => {
    expect(isEsmDependency('/proj/node_modules/x/index.ts')).toBe(false);
    expect(isEsmDependency('/proj/node_modules/x/index.json')).toBe(false);
  });

  it('does NOT treat app code outside node_modules as an ESM dep', () => {
    expect(isEsmDependency('/proj/src/app.ts')).toBe(false);
    expect(isEsmDependency('/proj/src/app.js')).toBe(false);
  });

  it('honors processEsmModules regex sources', () => {
    expect(isEsmDependency('/proj/src/legacy-esm.js', ['legacy-esm'])).toBe(true);
    expect(isEsmDependency('/proj/src/app.ts', ['legacy-esm'])).toBe(false);
  });
});
