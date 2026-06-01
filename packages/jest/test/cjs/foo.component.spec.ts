import { FooComponent } from './fixtures/foo.component';

describe('@oxc-angular-testing/jest — CommonJS', () => {
  it('transforms ESM TypeScript to runnable CommonJS and inlines templateUrl', () => {
    // This whole file (and the imported component) were emitted as CommonJS
    // (require/exports + esModuleInterop) by the transform, then run by classic
    // CJS jest.
    const meta = (FooComponent as unknown as { __annotations__: any[] }).__annotations__[0];
    expect(meta.template).toBe('<h1>Hello from CommonJS</h1>\n');
    expect(meta.templateUrl).toBeUndefined();
    expect(meta.styleUrls).toBeUndefined();
  });

  it('produces an instantiable class', () => {
    expect(new FooComponent().title).toBe('hi');
  });
});
