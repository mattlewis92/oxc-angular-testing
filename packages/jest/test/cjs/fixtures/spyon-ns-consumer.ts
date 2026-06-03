import * as dep from './spyon-dep';
// Reads the member at call time through the namespace object.
export function callGreet(): string {
  return dep.greet();
}
