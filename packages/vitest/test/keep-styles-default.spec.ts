import { describe, expect, it } from 'vitest';
import { StyledComponent } from './fixtures/styled.component';

describe('keepStyles default outside browser mode', () => {
  it('strips styleUrl and inline styles in node and jsdom projects', () => {
    // Runs in both the `node` (ssr environment) and `jsdom` (client
    // environment) projects: neither is browser mode, so no `?inline` import
    // of the scss is emitted and the metadata carries no styles.
    const meta = (StyledComponent as unknown as { __annotations__: Record<string, unknown>[] })
      .__annotations__[0]!;
    expect(meta.styleUrl).toBeUndefined();
    expect(meta.styles).toBeUndefined();
    expect(meta.template).toBe('<div class="styled"></div>');
  });
});
