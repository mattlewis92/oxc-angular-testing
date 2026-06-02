import { Greeting } from './fixtures/Greeting';

// Greeting.tsx is compiled by the transform (automatic JSX runtime → requires
// `react/jsx-runtime`, stubbed via moduleNameMapper). Calling the component
// returns the runtime's element object — proving .tsx compiles and runs.
describe('@oxc-angular-testing/jest — TSX (React) support', () => {
  it('compiles a .tsx React component via the automatic JSX runtime', () => {
    const el = Greeting({ name: 'world' }) as unknown as {
      type: string;
      props: { className: string; children: unknown[] };
    };
    expect(el.type).toBe('span');
    expect(el.props.className).toBe('greeting');
    expect(el.props.children).toEqual(['Hello ', 'world']);
  });
});
