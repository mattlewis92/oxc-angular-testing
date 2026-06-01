import { describe, expect, it } from 'vitest';
import { Dep, SignalComponent } from './fixtures/signal.component';

type Annotated = {
  __annotations__: any[];
  propDecorators: Record<string, Array<{ type: unknown; args?: any[] }>>;
  ctorParameters: () => Array<{ type: unknown }>;
};

const klass = SignalComponent as unknown as Annotated;

describe('@oxc-angular-testing/vitest — Angular JIT transforms', () => {
  it('downlevels signal input() into propDecorators with @Input metadata', () => {
    const input = klass.propDecorators.disabled[0];
    expect(input.args?.[0].isSignal).toBe(true);
    expect(input.args?.[0].alias).toBe('disabled');
    expect(input.args?.[0].required).toBe(false);
  });

  it('downlevels signal output() into propDecorators with @Output(name)', () => {
    expect(klass.propDecorators.changed[0].args?.[0]).toBe('changed');
  });

  it('downlevels constructor params into ctorParameters (DI metadata)', () => {
    const params = klass.ctorParameters();
    expect(params[0].type).toBe(Dep);
  });

  it('runs the @Component decorator with the inlined template', () => {
    expect(klass.__annotations__[0].template).toBe('<p>sig</p>');
  });

  it('instantiates with an injected dependency', () => {
    const component = new SignalComponent(new Dep());
    expect(component.dep.value).toBe(42);
  });
});
