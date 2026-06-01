import { NgThing, VERSION } from './fixtures/ng-like.mjs';
describe('CJS jest loads a transformed .mjs dependency', () => {
  it('downlevels .mjs to CJS and requires it', () => {
    expect(VERSION).toBe('0.0.0-mjs');
    expect(new NgThing().hello()).toBe('ng:0.0.0-mjs');
  });
});
