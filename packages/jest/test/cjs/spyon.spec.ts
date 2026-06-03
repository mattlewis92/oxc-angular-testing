import * as dep from './fixtures/spyon-dep';
import * as barrel from './fixtures/spyon-barrel';
import { callGreet } from './fixtures/spyon-ns-consumer';
import { callViaBarrel } from './fixtures/spyon-barrel-consumer';

// Behavioral canary for the two intentional configurable-getter deviations from tsc:
// the `__importStar`/`__createBinding` namespace shim (configurable + settable) and
// `define_export_getter` (configurable). These exist so `jest.spyOn` can redefine a
// namespace member / re-exported binding; today that is only string-asserted (the
// emitted code contains `configurable: true`). Here we actually run jest.spyOn against
// the compiled CJS output and assert the spy is observed by a separate consumer module
// — which only works if the getter is configurable/settable and writes through to the
// shared source module.
describe('@oxc-angular-testing/jest — jest.spyOn over namespace / re-export members', () => {
  afterEach(() => jest.restoreAllMocks());

  it('intercepts a namespace-import member (import * as ns)', () => {
    expect(callGreet()).toBe('real');
    jest.spyOn(dep, 'greet').mockReturnValue('mocked-ns');
    // The consumer reads dep.greet() at call time through its own namespace view; the
    // spy must be visible there too (the settable getter writes back to the source).
    expect(callGreet()).toBe('mocked-ns');
  });

  it('intercepts a re-export-barrel member (export { x } from ...)', () => {
    expect(callViaBarrel()).toBe('real');
    jest.spyOn(barrel, 'greet').mockReturnValue('mocked-barrel');
    expect(callViaBarrel()).toBe('mocked-barrel');
  });
});
