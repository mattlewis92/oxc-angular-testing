import { greet } from './spyon-barrel';
export function callViaBarrel(): string {
  return greet();
}
