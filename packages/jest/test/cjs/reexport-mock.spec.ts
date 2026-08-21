import { realFn } from './fixtures/reexport-barrel';

// A barrel that BOTH default-imports and named-re-exports the same module,
// where that module is replaced by a `jest.mock` factory WITHOUT
// `__esModule: true` (the common way people write factories). The re-export
// getter must read the RAW module namespace: if it read a shared
// `__importDefault`-wrapped variable, the missing `__esModule` marker would
// wrap the mock as `{ default: mock }` and every named re-export would come
// back `undefined`. tsc emits a separate raw require for the re-export; so do we.
jest.mock('./fixtures/reexport-dep', () => ({
  realFn: jest.fn(() => 'mocked'),
}));

describe('@oxc-angular-testing/jest — named re-export of a jest.mock without __esModule', () => {
  it('resolves the named re-export to the mock, not undefined', () => {
    expect(typeof realFn).toBe('function');
    expect(realFn()).toBe('mocked');
  });
});
