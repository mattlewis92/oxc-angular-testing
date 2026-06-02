import { greet } from './fixtures/greeter';

// Written AFTER the import on purpose: only works if the transform hoists
// jest.mock above the import (babel-plugin-jest-hoist behavior).
jest.mock('./fixtures/greeter', () => ({ greet: () => 'mocked' }));

describe('@oxc-angular-testing/jest — jest.mock hoisting', () => {
  it('applies a mock declared after the import', () => {
    expect(greet()).toBe('mocked');
  });
});
