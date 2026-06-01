// Minimal stand-in for @angular/core: the decorators record their metadata on
// the class so the integration test can assert what the transform produced,
// without pulling in the real Angular runtime.

export function Component(meta) {
  return function (target) {
    target.__annotations__ = (target.__annotations__ || []).concat([meta]);
    return target;
  };
}

export function Input() {
  return function () {};
}

export function Output() {
  return function () {};
}

export function input(initial) {
  const fn = () => initial;
  fn.required = () => () => undefined;
  return fn;
}

export function output() {
  return { emit() {} };
}
