// Minimal stand-in for @angular/core (see the vitest package for the rationale).

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
