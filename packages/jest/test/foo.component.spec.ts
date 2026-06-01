import { FooComponent } from './fixtures/foo.component';

describe('@oxc-angular-testing/jest', () => {
  it('inlines templateUrl into the @Component metadata and strips styles', () => {
    const meta = (FooComponent as unknown as { __annotations__: any[] }).__annotations__[0];
    expect(meta.template).toBe('<h1>Hello from jest</h1>\n');
    expect(meta.templateUrl).toBeUndefined();
    expect(meta.styleUrls).toBeUndefined();
    expect(meta.selector).toBe('app-foo');
  });

  it('produces an instantiable class with types stripped', () => {
    expect(new FooComponent().title).toBe('hi');
  });
});
