import { describe, expect, it } from 'vitest';
import { FooComponent } from './fixtures/foo.component';

describe('@oxc-angular-testing/vitest', () => {
  it('inlines templateUrl into the @Component metadata and strips styles', () => {
    // The transform replaced `templateUrl` with the HTML content (loaded by the
    // plugin), lowered the decorator (so it executed), and stripped styles. The
    // fake @angular/core recorded the metadata on the class.
    const meta = (FooComponent as unknown as { __annotations__: any[] }).__annotations__[0];
    expect(meta.template).toBe('<h1>Hello from template</h1>\n');
    expect(meta.templateUrl).toBeUndefined();
    expect(meta.styleUrls).toBeUndefined();
    expect(meta.selector).toBe('app-foo');
  });

  it('produces an instantiable class with types stripped', () => {
    const instance = new FooComponent();
    expect(instance.title).toBe('hi');
  });
});
