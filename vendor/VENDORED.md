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

1. **`src/instrument.rs`** — added one public function `instrument_program(allocator,
   program, source, filename, options)`: the post-parse half of `instrument()`,
   operating on a program the caller already parsed/transformed in their arena.
   It does not parse and does not run the TS-strip pass. No existing code changed.
2. **`src/lib.rs`** — re-export `instrument_program`.
3. **`Cargo.toml`** — made self-contained: literal `[package]` fields (was
   `*.workspace = true`), sibling deps repointed to crates.io, `[lints]`,
   `[[bench]]`, and `[dev-dependencies]` removed (not needed for our use).
4. **`src/transform.rs`** — skip **synthesized** functions/arrows (zero-width
   span) when building `fnMap`, and fall back to the decl span when a function's
   body span is zero. We instrument *after* the Angular/TS transforms, which add
   compiler-generated functions with no source location (the constructor oxc
   synthesizes for class-field init under `useDefineForClassFields: false`; the
   `ctorParameters = () => […]` arrow; the dynamic-import `() => …` wrapper). The
   real istanbul never instruments generated code, so counting these inflated our
   function coverage vs a babel/jest-preset-angular setup. The function entry is
   skipped but real-span statements inside the body are still counted. The crate's
   own tests (all real-span inputs) are unaffected.

No other `src/*.rs` file was modified.

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
3. Re-apply `expose-transform.patch` (or re-add `instrument_program` + the lib.rs
   re-export by hand — it is ~80 lines mirroring `instrument()`).
4. Refresh the self-contained `Cargo.toml` (bump oxc/sibling versions to match the
   new tag's manifest), then bump `oxc_* = "=<new>"` across our crates in lockstep.
5. Update the tag/commit above.
