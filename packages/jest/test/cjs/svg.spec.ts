import logo from './fixtures/logo.svg';

describe('@oxc-angular-testing/jest — SVG stringification (CommonJS)', () => {
  it('imports an .svg as its raw string content (not compiled as code)', () => {
    expect(typeof logo).toBe('string');
    expect(logo).toContain('<svg');
    expect(logo).toContain('<rect');
  });
});
