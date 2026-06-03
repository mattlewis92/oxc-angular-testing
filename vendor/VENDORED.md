# Vendored: `oxc_coverage_instrument`

We run istanbul-compatible coverage instrumentation **on the same parsed AST**
that the Angular transforms mutate, so the whole pipeline shares one parse and
one codegen. The upstream crate only exposes a `instrument(source, …)` entry
that re-parses internally, so we vendor its source and add a single post-parse
entry point.

## Source

- Repo: <https://github.com/fallow-rs/oxc-coverage-instrument>
- Tag: **v0.7.6**
- Commit: **8faba6df9f994c911401ad394b50351b8ca7f943**
- Path taken: `crates/oxc-coverage-instrument/{src,Cargo.toml}`

Only this one crate's `src/` is vendored. Its sibling suite crates
(`oxc_coverage_types`, `oxc_coverage_source_maps`, `oxc_coverage_v8`) are pulled
unmodified from crates.io.

## Local changes (see `expose-transform.patch`)

1. **`src/instrument.rs`** — added one public function on top of `instrument()`:
   - `instrument_program_ast(...)` → `InstrumentAstResult { coverage_map_json,
     preamble }`: instrument **without** codegen — insert the counters into the
     program and return the coverage map + preamble text. This lets us instrument
     at *source level* (before the Angular/TS transforms) and codegen once at the
     end, so the coverage map mirrors the source: independent of the ES `target`
     (no `?.`/`??`/`async` branch reshaping) and free of compiler-synthesized
     nodes (the field-init constructor, `ctorParameters` arrows, the
     dynamic-import wrapper). No existing code changed.
2. **`src/lib.rs`** — re-export `instrument_program_ast` and `InstrumentAstResult`.
3. **`Cargo.toml`** — made self-contained: literal `[package]` fields (was
   `*.workspace = true`), sibling deps repointed to crates.io, `[lints]`,
   `[[bench]]`, and `[dev-dependencies]` removed (not needed for our use).

### Bug fix (not in `expose-transform.patch`)

4. **`src/transform.rs`** (`exit_statements`) — upstream drops the per-declarator
   statement counter for an **exported** fn/arrow/class-init declarator
   (`export const f = () => …`). The counter is hoisted to a sibling before the
   enclosing `VariableDeclaration`, but the body statement is the wrapping
   `ExportNamedDeclaration` (span starts at `export`, not `const`), so the
   offsets never match and `++s[N]` is silently dropped — the declaration is
   never counted (coverage reports 1/2 instead of 2/2). Fix: when matching a
   pending insertion, also accept an `ExportNamedDeclaration`'s inner declaration
   start, so the counter lands before the whole `export` statement (matching
   istanbul's `cov.s[N]++; export const f = …`). Re-apply on re-sync; consider
   upstreaming.

5. **`src/transform.rs`** (`generate_preamble_source`) — splice istanbul's
   `_coverageSchema` marker (bare-identifier key `_coverageSchema:"1a1c01bbd47…"`)
   into the head of the emitted `coverageData` object literal. Upstream's preamble
   embeds the `FileCoverage` JSON with no schema key, so
   `istanbul-lib-instrument`'s `readInitialCoverage` (used by jest's
   `generateEmptyCoverage` to report never-imported `collectCoverageFrom` files as
   0%) can't locate the coverage object → those files are dropped from the report.
   The key must be a JS identifier (not a JSON string), so it's spliced into the
   literal source. Re-apply on re-sync; consider upstreaming.

6. **`src/transform.rs`** (`enter_call_expression`) — for an OPTIONAL call
   (`obj?.method?.()`) whose callee is a member expression, do NOT wrap the callee
   with the `cov_oc` link observer. Wrapping it (`cov_oc(obj?.method, id)?.()`)
   evaluates the callee to a detached function value, so the method runs with
   `this === undefined` (R22 — broke any instrumented `obj?.m?.()` using `this`).
   The member link's own branch already records the object's short-circuit, so the
   call-link counter is dropped for method calls; a non-member callee (`fn?.()`)
   has no receiver to lose and is still wrapped. Re-apply on re-sync.

   **Known coverage gap (intentional):** because the skip keys off *any* member-
   expression callee, `obj.method?.()` — where the member is non-optional and the
   only `?.` is the call's own — records **no** optional-chain branch: the call-link
   is skipped, and the static-member visitor only records when the member itself is
   optional. We accept this rather than wrap it, because wrapping `obj.method?.()`
   would detach `this` exactly as in `obj?.method?.()` (any wrapped member callee
   loses its receiver). Statement coverage for the call is unaffected — only that
   one optional-call *branch* counter is missing. Recording it correctly would need
   a receiver-preserving rewrite (a temp-bound receiver), not the `cov_oc` wrapper.

7. **`src/transform.rs`** (`generate_preamble_source`) — `debug_assert!` that the
   serialized coverage JSON starts with `{` before the `_coverageSchema` splice, so
   the else-branch (which would emit the preamble WITHOUT the marker, silently
   re-breaking never-imported-file coverage) can't be taken unnoticed in test builds.
   Test-only; no release behavior change. Re-apply on re-sync.

No other `src/*.rs` change.

## Wiring

Root `Cargo.toml`:

```toml
[patch.crates-io]
oxc_coverage_instrument = { path = "vendor/oxc-coverage-instrument" }
```

The directory is listed in the workspace `exclude` so it is not auto-added as a
member (it builds only as a patch target).

## Re-syncing on a version bump

1. `git clone --branch <new-tag> https://github.com/fallow-rs/oxc-coverage-instrument`
2. Copy `crates/oxc-coverage-instrument/src` over `vendor/oxc-coverage-instrument/src`.
3. Re-apply `expose-transform.patch` (or re-add `instrument_program_ast` +
   `InstrumentAstResult` + the lib.rs re-export by hand — it is ~70 lines mirroring
   `instrument()` minus its codegen tail). The patch's `@@` line numbers track
   upstream v0.7.6; on a version bump re-apply by hand, then regenerate the patch
   with `diff -u <upstream>/src/{lib,instrument}.rs vendor/oxc-coverage-instrument/src/{lib,instrument}.rs`.
4. Refresh the self-contained `Cargo.toml` (bump oxc/sibling versions to match the
   new tag's manifest), then bump `oxc_* = "=<new>"` across our crates in lockstep.
5. Update the tag/commit above.
