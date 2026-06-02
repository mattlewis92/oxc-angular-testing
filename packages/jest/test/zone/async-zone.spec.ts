// R10 regression: with `tsconfig: '<rootDir>/...'` (the standard jest pattern), the
// plugin must expand <rootDir>, derive target es2016, and DOWNLEVEL async — so the
// result is built via the helper's live-global `new Promise` = the zone-patched
// ZoneAwarePromise. Before the <rootDir>-expansion fix, target wasn't derived, async
// stayed native, and the result was a non-zone native Promise (instanceof false).
describe('R10 — async under zone is the zone-patched Promise (via <rootDir> tsconfig)', () => {
  async function makeAsyncInModule(): Promise<number> {
    return 1;
  }
  it('global Promise is zone-patched', () => {
    expect((globalThis.Promise as unknown as { name: string }).name).toBe('ZoneAwarePromise');
  });
  it('async result is instanceof the zone-patched global Promise', () => {
    expect(makeAsyncInModule() instanceof Promise).toBe(true);
  });
});
