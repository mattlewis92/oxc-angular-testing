# Vendored: `oxc_coverage_instrument`

We run istanbul-compatible coverage instrumentation **on the same parsed AST**
that the Angular transforms mutate, so the whole pipeline shares one parse and
one codegen. The upstream crate only exposes a `instrument(source, …)` entry
that re-parses internally, so we vendor its source and add a single post-parse
entry point.

## Source

- Repo: <https://github.com/fallow-rs/oxc-coverage-instrument>
- Tag: **v0.9.0**
- Commit: **bce77875e81522bdb582456b394154cbdefdedc8**
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
     dynamic-import wrapper). No existing code changed. It intentionally does **not**
     surface unhandled pragmas (unlike `instrument()`'s `InstrumentResult`): there is
     no codegen here and the host has no warnings channel, so an unprocessed
     `/* istanbul ignore */` is dropped. Add an `unhandled_pragmas` field if that ever
     needs surfacing. It mirrors `instrument()`'s `TransformInit` construction, so on
     a re-sync keep its fields in lockstep (as of v0.9.0: `allocator`, `source`,
     `cov_fn_name`, `report_logic`, `track_optional_chain`, `ignore_class_methods`,
     `eager_remapper`) and use `finalize_coverage_map(filename, transform, options)`.
2. **`src/lib.rs`** — re-export `instrument_program_ast` and `InstrumentAstResult`.
3. **`Cargo.toml`** — made self-contained: literal `[package]` fields (was
   `*.workspace = true`), sibling deps repointed to crates.io, `[lints]`,
   `[[bench]]`, and `[dev-dependencies]` removed (not needed for our use).

### Bug fix (not in `expose-transform.patch`)

> Note: the exported-fn-init-declarator counter fix we previously carried here was
> **upstreamed in v0.8.2** (the `ExportNamedDeclarationDeclaration` ancestor branch in
> `transform.rs`'s declarator handling), so it is no longer a local change.

4. **`src/transform.rs`** (`generate_preamble_source`) — splice istanbul's
   `_coverageSchema` marker (bare-identifier key `_coverageSchema:"1a1c01bbd47…"`)
   into the head of the emitted `coverageData` object literal. Upstream's preamble
   embeds the `FileCoverage` JSON with no schema key, so
   `istanbul-lib-instrument`'s `readInitialCoverage` (used by jest's
   `generateEmptyCoverage` to report never-imported `collectCoverageFrom` files as
   0%) can't locate the coverage object → those files are dropped from the report.
   The key must be a JS identifier (not a JSON string), so it's spliced into the
   literal source. Re-apply on re-sync; consider upstreaming.

5. **`src/transform.rs`** (`enter_call_expression`) — for an OPTIONAL call
   (`obj?.method?.()`) whose callee is a member expression, do NOT wrap the callee
   with the `cov_oc` link observer. Wrapping it (`cov_oc(obj?.method, id)?.()`)
   evaluates the callee to a detached function value, so the method runs with
   `this === undefined` (R22 — broke any instrumented `obj?.m?.()` using `this`).
   The member link's own branch already records the object's short-circuit, so the
   call-link counter is dropped for method calls; a non-member callee (`fn?.()`)
   has no receiver to lose and is still wrapped. **Re-apply on re-sync — upstream
   v0.9.0 reverted to wrapping all callees, so this must be re-added each bump.**

   **Known coverage gap (intentional):** because the skip keys off *any* member-
   expression callee, `obj.method?.()` — where the member is non-optional and the
   only `?.` is the call's own — records **no** optional-chain branch: the call-link
   is skipped, and the static-member visitor only records when the member itself is
   optional. We accept this rather than wrap it, because wrapping `obj.method?.()`
   would detach `this` exactly as in `obj?.method?.()` (any wrapped member callee
   loses its receiver). Statement coverage for the call is unaffected — only that
   one optional-call *branch* counter is missing. Recording it correctly would need
   a receiver-preserving rewrite (a temp-bound receiver), not the `cov_oc` wrapper.

6. **`src/transform.rs`** (`generate_preamble_source`) — `debug_assert!` that the
   serialized coverage JSON starts with `{` before the `_coverageSchema` splice, so
   the else-branch (which would emit the preamble WITHOUT the marker, silently
   re-breaking never-imported-file coverage) can't be taken unnoticed in test builds.
   Test-only; no release behavior change. Re-apply on re-sync.

No other `src/*.rs` change.

## What the host actually consumes

The host imports exactly **two** items from this crate:
`oxc_coverage_instrument::{InstrumentOptions, instrument_program_ast}` (see
`crates/ng-transform/src/lib.rs`). Everything else the crate exposes — the original
re-parsing `instrument()` entry, `v8_to_istanbul`, every `remap_coverage*` source-map
function, the `function_identity_overlay` / compose / remap paths — is **inert** from
our perspective. It is kept only to vendor the crate faithfully (clippy stays quiet
because it's all `pub`); none of it is wired into the pipeline. A future reader
chasing callers of those exports will find none — that is expected, not a bug.

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
3. Re-apply the **3 transform.rs patches** by hand (items 4–6 above: the
   `_coverageSchema` splice, the R22 member-callee skip — upstream keeps reverting it,
   and the `debug_assert`) plus the `instrument_program_ast`/`InstrumentAstResult`
   additions + the lib.rs re-export (`expose-transform.patch` is the reference). Keep
   `instrument_program_ast`'s `TransformInit` fields and coverage-map call
   (`finalize_coverage_map`) in lockstep with the current `instrument()` (see item 1).
   The patch's `@@` line numbers track the pinned tag; regenerate it after re-applying
   with `diff -u <upstream>/src/{lib,instrument}.rs vendor/oxc-coverage-instrument/src/{lib,instrument}.rs`.
4. Refresh the self-contained `Cargo.toml` (bump oxc/sibling versions to match the
   new tag's manifest). If the new tag changes its required oxc version, bump
   `oxc_* = "=<new>"` across our crates in lockstep too (v0.9.0 still uses 0.126).
5. **Bump the version requirement in the root `Cargo.toml`'s `[workspace.dependencies]`
   `oxc_coverage_instrument = "<new>"`** to match the vendored crate's version.
   Otherwise the `[patch.crates-io]` entry doesn't satisfy the requirement and Cargo
   **silently ignores the patch**, pulling the unpatched crate from crates.io (you'll
   get "no `instrument_program_ast` in the root").
6. Update the tag/commit above. Re-verify the istanbul coverage differential
   (`crates/ng-transform-napi/test/coverage-differential.test.mts`) — upstream counter
   refactors can shift counts.
