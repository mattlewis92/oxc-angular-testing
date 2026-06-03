//! End-to-end smoke tests: the pipeline parses, codegens, and (when requested)
//! folds istanbul coverage into the same pass via the vendored
//! `instrument_program_ast`.

use ng_transform::{TransformOptions, transform};

#[test]
fn plain_transform_returns_code() {
    // Default options are CommonJS → `export const` becomes `exports.x`.
    let out = transform(
        "export const x = 1 + 2;",
        "x.ts",
        &TransformOptions::default(),
    );
    assert!(out.errors.is_empty(), "errors: {:?}", out.errors);
    assert!(out.code.contains("exports.x = x"), "code:\n{}", out.code);
    assert!(out.coverage_map.is_none());
}

#[test]
fn coverage_instruments_in_one_pass() {
    let opts = TransformOptions {
        coverage: true,
        ..TransformOptions::default()
    };
    let out = transform("function add(a, b) { return a + b; }", "add.js", &opts);
    assert!(out.errors.is_empty(), "errors: {:?}", out.errors);
    // Preamble + counter wiring from oxc_coverage_instrument.
    assert!(out.code.contains("__coverage__"), "code:\n{}", out.code);
    let map = out.coverage_map.expect("coverage map present");
    assert!(map.contains("fnMap"), "coverage map: {map}");
    assert!(map.contains("statementMap"));
}

#[test]
fn coverage_preamble_carries_the_istanbul_schema_marker() {
    // istanbul's `readInitialCoverage` (used by jest's generateEmptyCoverage to
    // report never-imported `collectCoverageFrom` files as 0%) locates our coverage
    // object by the bare-identifier key `_coverageSchema: "<sha1>"`. Guard our own
    // emission of it; the JS plugins separately verify the value still matches the
    // consumer's installed istanbul at runtime.
    let opts = TransformOptions {
        coverage: true,
        jit_transforms: false,
        ..TransformOptions::default()
    };
    let out = transform("export const x = 1;\n", "x.ts", &opts);
    assert!(out.errors.is_empty(), "errors: {:?}", out.errors);
    assert!(
        out.code
            .contains(r#"_coverageSchema:"1a1c01bbd47fc00a2c39e90264f33305004495a9""#),
        "emitted preamble must carry istanbul's _coverageSchema marker:\n{}",
        out.code
    );
}
