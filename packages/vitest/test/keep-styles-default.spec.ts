import { describe, expect, it } from 'vitest';
import { StyledComponent } from './fixtures/styled.component';

describe('keepStyles default outside browser mode', () => {
  it('strips styleUrl and inline styles in the node (ssr) environment', () => {
    // This suite runs in the `ssr` Vite environment (plain node, the jsdom-style
    // unit-test setup), so the environment-based `keepStyles` default must keep
    // the historical stripping behavior — no `?inline` import of the scss is
    // emitted (the file is never loaded) and the metadata carries no styles.
    const meta = (StyledComponent as unknown as { __annotations__: Record<string, unknown>[] })
      .__annotations__[0]!;
    expect(meta.styleUrl).toBeUndefined();
    expect(meta.styles).toBeUndefined();
    expect(meta.template).toBe('<div class="styled"></div>');
  });
});
