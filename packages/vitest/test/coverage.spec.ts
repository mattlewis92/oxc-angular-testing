import { describe, expect, it } from 'vitest';
import oxcAngular from '../src/index.ts';

// The transform/load hooks are object-form (`{ filter, handler }`); grab the handler.
function handlerOf(hook: any) {
  return typeof hook === 'function' ? hook : hook.handler;
}

const ctx = {
  error(message: string): never {
    throw new Error(message);
  },
};

describe('@oxc-angular-testing/vitest — coverage auto-enable', () => {
  it('instruments when vitest runs with the istanbul coverage provider', () => {
    const plugin: any = oxcAngular();
    plugin.configResolved({ test: { coverage: { enabled: true, provider: 'istanbul' } } });
    const out = handlerOf(plugin.transform).call(ctx, 'export const x = 1;', '/proj/x.ts');
    expect(out.code).toContain('__coverage__');
  });

  it('does not instrument when coverage is disabled', () => {
    const plugin: any = oxcAngular();
    plugin.configResolved({ test: {} });
    const out = handlerOf(plugin.transform).call(ctx, 'export const x = 1;', '/proj/x.ts');
    expect(out.code).not.toContain('__coverage__');
  });

  it('does not auto-instrument for the v8 provider', () => {
    const plugin: any = oxcAngular();
    plugin.configResolved({ test: { coverage: { enabled: true, provider: 'v8' } } });
    const out = handlerOf(plugin.transform).call(ctx, 'export const x = 1;', '/proj/x.ts');
    expect(out.code).not.toContain('__coverage__');
  });

  it('explicit coverage option overrides auto-detection', () => {
    const plugin: any = oxcAngular({ coverage: true });
    plugin.configResolved({ test: {} }); // coverage disabled in vitest
    const out = handlerOf(plugin.transform).call(ctx, 'export const x = 1;', '/proj/x.ts');
    expect(out.code).toContain('__coverage__');
  });
});
