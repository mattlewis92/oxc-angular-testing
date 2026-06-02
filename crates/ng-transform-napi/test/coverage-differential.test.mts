import assert from 'node:assert/strict';
import { test } from 'node:test';
import istanbul from 'istanbul-lib-instrument';
import { transform } from '../index.js';

// Differential coverage test: instrument each snippet with BOTH our transform and
// the canonical `istanbul-lib-instrument` (the same engine jest/babel-plugin-istanbul
// use), execute both with the identical exercise, and assert the resulting coverage
// is equivalent — same statement/function/branch *locations* and the same *hit
// counts*. This is the regression net for the whole class of coverage bugs that a
// single hand-written assertion can't cover: dropped counters (undercount),
// duplicated counters (overcount), wrong attribution (a counter on the wrong line),
// and map-structure drift from istanbul.
//
// Both sides are emitted and run as real ESM (via a `data:` import) so the
// comparison is symmetric. The corpus is plain JS + ESM syntax only: TS-specific
// lowering (enums, parameter properties, decorators) has no istanbul oracle —
// istanbul does not lower TS — so it is covered by the hand-written tests instead.
//
// Three intentional, documented deviations are normalized out — none affect a
// coverage count, percentage, or covered/total (verified across the corpus):
//   1. fnMap `name`: istanbul labels arrows/methods `(anonymous_N)`; we infer the
//      binding/member name (`f`, `m`) — more useful in reports.
//   2. fnMap `decl` span: istanbul points at the `function`/`get`/`set` keyword; we
//      point at the function/member name. The function body `loc` (which carries the
//      hit count) is identical.
//   3. branch `locations` sub-spans, specifically the implicit-`else` of an
//      else-less `if`: istanbul stores it as an `undefined` location, we store a
//      zero-width span. The branch's own `loc` and hit array are identical.
// Separately, OUR instrument emits optional-chain branch coverage that
// istanbul-lib-instrument 6.x does not (a deliberate, target-independent choice —
// see the `branch coverage shape is independent of the ES target` test). Those are
// excluded from the istanbul diff and asserted on their own below.

const LOGICAL = 'snippet.js';

const inst = istanbul.createInstrumenter({
  esModules: true,
  coverageVariable: '__coverage__',
  compact: true,
  produceSourceMap: false,
});

// `data:` module imports are content-cached by the ESM loader, so two snippets with
// identical source (e.g. an if/else exercised then-path vs else-path) would reuse the
// first module — whose coverage closure keeps accumulating and ignores the reset
// below. A unique trailing comment per call makes every module distinct. (A trailing
// comment can't shift the baked-in coverage map or affect execution.)
let nonce = 0;
async function instrumentAndRun(
  code: string,
  run: (mod: any) => void,
): Promise<any> {
  (globalThis as any).__coverage__ = {};
  const unique = `${code}\n//cache-buster:${nonce++}`;
  const mod = await import('data:text/javascript,' + encodeURIComponent(unique));
  run(mod);
  return (globalThis as any).__coverage__[LOGICAL];
}

const ours = (src: string) =>
  transform(src, LOGICAL, { module: 'esm', coverage: true, jitTransforms: false }).code;
const theirs = (src: string) => inst.instrumentSync(src, LOGICAL);

const span = (loc: any) =>
  loc && loc.start.line != null
    ? `${loc.start.line}:${loc.start.column}-${loc.end.line}:${loc.end.column}`
    : '∅';

// Reduce a coverage object to a location-keyed, index-independent profile: every
// statement/function/branch identified by its source span and paired with its hit
// count(s). Equivalent coverage ⇒ deeply-equal profiles. The documented deviations
// (fn name/decl, implicit-else location, optional-chain branches) are normalized out
// here — see the header.
function profile(cov: any) {
  const statements = Object.keys(cov.statementMap)
    .map((id) => [span(cov.statementMap[id]), cov.s[id]] as const)
    .sort();
  // Functions keyed by body `loc` (carries the hit count); `decl`/`name` excluded.
  const functions = Object.keys(cov.fnMap)
    .map((id) => [span(cov.fnMap[id].loc), cov.f[id]] as const)
    .sort();
  // Branches keyed by type + own `loc` + hit array; inner `locations` excluded
  // (implicit-else representation). Optional-chain branches excluded (ours-only).
  const branches = Object.keys(cov.branchMap)
    .filter((id) => cov.branchMap[id].type !== 'optional-chain')
    .map((id) => [`${cov.branchMap[id].type}@${span(cov.branchMap[id].loc)}`, JSON.stringify(cov.b[id])] as const)
    .sort();
  return { statements, functions, branches };
}

// Per-snippet covered/total must also agree — this is what a coverage threshold
// actually gates on. Branch totals exclude optional-chain (the ours-only enhancement)
// so the comparison is against what istanbul measures.
function totals(cov: any) {
  const coveredCount = (o: Record<string, number>) =>
    Object.values(o).filter((n) => n > 0).length;
  let bCovered = 0;
  let bTotal = 0;
  for (const id of Object.keys(cov.branchMap)) {
    if (cov.branchMap[id].type === 'optional-chain') continue;
    const arr = cov.b[id];
    bTotal += arr.length;
    bCovered += arr.filter((n) => n > 0).length;
  }
  return {
    statements: { covered: coveredCount(cov.s), total: Object.keys(cov.s).length },
    functions: { covered: coveredCount(cov.f), total: Object.keys(cov.f).length },
    branches: { covered: bCovered, total: bTotal },
  };
}

// Corpus: [name, source, exercise]. The same `exercise` runs against both
// instrumented builds (they export the same shape), so any count difference is a
// real instrumentation divergence — not a difference in how they were driven.
// Partial exercises (leaving some branches/statements untaken) make the hit-count
// comparison meaningful rather than all-ones.
const CORPUS: Array<[string, string, (m: any) => void]> = [
  // — export / declaration wrappers (the regression area) —
  ['export const arrow (implicit)', `export const f = (x) => x * 2;`, (m) => m.f(3)],
  ['export const arrow (block)', `export const f = (x) => { return x * 2; };`, (m) => m.f(3)],
  ['export let fn-init', `export let f = () => 1;`, (m) => m.f()],
  ['export const multi-declarator', `export const a = () => 1, b = () => 2;`, (m) => m.a()],
  ['export function decl', `export function f(x) { return x * 2; }`, (m) => m.f(3)],
  ['export default arrow', `export default () => 1;`, (m) => m.default()],
  ['export default function', `export default function () { return 1; }`, (m) => m.default()],
  ['export default class', `export default class { m() { return 1; } }`, (m) => new m.default().m()],
  ['export class + method', `export class C { m(x) { return x; } }`, (m) => new m.C().m(1)],

  // — statements —
  ['const + reassign + expr-stmt', `export let n = 0;\nn = n + 1;\nexport const get = () => n;`, (m) => m.get()],
  ['if / else taken-then', `export function f(x){ if (x) { return 1; } return 2; }`, (m) => m.f(1)],
  ['if / else taken-else', `export function f(x){ if (x) { return 1; } return 2; }`, (m) => m.f(0)],
  ['for loop body run', `export function f(n){ let t=0; for(let i=0;i<n;i++){ t+=i; } return t; }`, (m) => m.f(3)],
  ['for loop body skipped', `export function f(n){ let t=0; for(let i=0;i<n;i++){ t+=i; } return t; }`, (m) => m.f(0)],
  ['while loop', `export function f(n){ let t=0; while(n>0){ t++; n--; } return t; }`, (m) => m.f(2)],
  ['for-of', `export function f(xs){ let t=0; for (const x of xs) { t+=x; } return t; }`, (m) => m.f([1, 2])],
  ['try / catch (no throw)', `export function f(){ try { return 1; } catch(e){ return 2; } }`, (m) => m.f()],
  ['try / catch (throws)', `export function f(b){ try { if(b) throw new Error(); return 1; } catch(e){ return 2; } }`, (m) => m.f(true)],
  ['throw statement', `export function f(b){ if (b) throw new Error('x'); return 1; }`, (m) => m.f(false)],

  // — functions —
  ['async fn', `export async function f(){ return 1; }`, (m) => m.f()],
  ['async arrow', `export const f = async () => 1;`, (m) => m.f()],
  ['generator', `export function* g(){ yield 1; yield 2; }`, (m) => [...m.g()]],
  ['nested fns', `export function outer(){ function inner(){ return 1; } return inner(); }`, (m) => m.outer()],
  ['getter / setter', `export class C { #v=0; get v(){ return this.#v; } set v(x){ this.#v=x; } }`, (m) => { const c = new m.C(); c.v = 5; return c.v; }],
  ['static method', `export class C { static make(){ return new C(); } m(){ return 1; } }`, (m) => m.C.make().m()],
  ['IIFE', `export const x = (() => 1 + 2)();`, () => {}],

  // — branches —
  ['ternary (then)', `export const f = (x) => x ? 1 : 2;`, (m) => m.f(1)],
  ['ternary (else)', `export const f = (x) => x ? 1 : 2;`, (m) => m.f(0)],
  ['logical && (short-circuit)', `export const f = (a, b) => a && b;`, (m) => m.f(0, 1)],
  ['logical || (both seen)', `export const f = (a, b) => a || b;\nexport const g = (a,b)=>a||b;`, (m) => { m.f(0, 1); m.f(1, 0); }],
  ['nullish ??', `export const f = (a, b) => a ?? b;`, (m) => { m.f(null, 1); m.f(2, 3); }],
  ['optional chain', `export const f = (x) => x?.y?.z;`, (m) => { m.f(null); m.f({ y: null }); m.f({ y: { z: 1 } }); }],
  ['default param (used + given)', `export function f(x = 5){ return x; }\nexport function g(x=5){ return x; }`, (m) => { m.f(); m.g(2); }],
  ['switch (case + default)', `export function f(x){ switch(x){ case 1: return 'a'; default: return 'd'; } }`, (m) => { m.f(1); m.f(9); }],
  ['if / else-if chain', `export function f(x){ if(x>2) return 1; else if(x>1) return 2; return 3; }`, (m) => { m.f(5); m.f(0); }],
];

// The one deviation istanbul does NOT measure: optional-chain branch coverage. Assert
// it directly so the enhancement is covered — each `?.` is a 2-outcome branch, and
// exercising null / non-null hits both outcomes (istanbul 6.x emits no such branch).
test('optional-chain branch coverage (ours-only enhancement, istanbul omits it)', async () => {
  const cov = await instrumentAndRun(ours(`export const f = (x) => x?.y?.z;`), (m) => {
    m.f(null); // first `?.` short-circuits
    m.f({ y: null }); // second `?.` short-circuits
    m.f({ y: { z: 1 } }); // full chain
  });
  const types = Object.values(cov.branchMap).map((b: any) => b.type);
  assert.deepEqual(types, ['optional-chain', 'optional-chain'], 'two optional-chain branches emitted');
  // Both branches reached, both outcomes (short-circuit / continue) taken.
  for (const hits of Object.values(cov.b) as number[][]) {
    assert.ok(hits.length === 2 && hits.every((n) => n > 0), `both outcomes hit: ${JSON.stringify(hits)}`);
  }

  // And istanbul-lib-instrument really does omit them (guards the premise — if a
  // future istanbul adds optional-chain branches, revisit the exclusion above).
  const ist = await instrumentAndRun(theirs(`export const f = (x) => x?.y?.z;`), (m) => m.f({ y: { z: 1 } }));
  assert.equal(Object.keys(ist.branchMap).length, 0, 'istanbul 6.x emits no optional-chain branches');
});

for (const [name, src, run] of CORPUS) {
  test(`coverage matches istanbul-lib-instrument: ${name}`, async () => {
    const oursCov = await instrumentAndRun(ours(src), run);
    const istCov = await instrumentAndRun(theirs(src), run);

    assert.deepEqual(
      profile(oursCov),
      profile(istCov),
      `location/hit profile diverges from istanbul for: ${name}\n` +
        `ours: ${JSON.stringify(profile(oursCov))}\n` +
        `ist : ${JSON.stringify(profile(istCov))}`,
    );
    assert.deepEqual(
      totals(oursCov),
      totals(istCov),
      `covered/total diverges from istanbul for: ${name}`,
    );
  });
}
