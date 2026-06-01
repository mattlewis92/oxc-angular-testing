import { describe, expect, it } from 'vitest';
import logo from './fixtures/logo.svg';

describe('@oxc-angular-testing/vitest — SVG stringification', () => {
  it('loads an .svg as its raw string content', () => {
    expect(typeof logo).toBe('string');
    expect(logo).toContain('<svg');
  });
});
