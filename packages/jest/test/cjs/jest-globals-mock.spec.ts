import { jest } from '@jest/globals';
import { realFn } from './fixtures/reexport-dep';

// `jest` imported from '@jest/globals' instead of the global: the hoisted
// `jest.mock(...)` call references the module's require var after the CJS
// rewrite, so the hoist must keep that require ABOVE the call — hoisting it
// past the import is a "Cannot access 'globals_1' before initialization" TDZ
// ReferenceError at suite load.
jest.mock('./fixtures/reexport-dep', () => ({
  realFn: jest.fn(() => 'mocked-via-globals'),
}));

describe('@oxc-angular-testing/jest — jest.mock with jest imported from @jest/globals', () => {
  it('registers the mock without a TDZ error', () => {
    expect(realFn()).toBe('mocked-via-globals');
  });
});
