//! Source-map fidelity for the CJS path. The ESM→CJS rewrite prepends a text
//! prelude (`"use strict";` + interop helpers) after codegen and rebuilds
//! statements with fresh nodes, so without care every position drifts. These
//! tests decode the emitted map and assert real generated tokens resolve back
//! to their original line.

use ng_transform::{ModuleKind, TransformOptions, transform};
use oxc_sourcemap::SourceMap;

struct Decoded {
    code: String,
    map: SourceMap,
}

fn cjs_with_map(src: &str) -> Decoded {
    let opts = TransformOptions {
        module: ModuleKind::CommonJs,
        target: "es2022".to_string(),
        jit_transforms: false,
        source_map: true,
        ..TransformOptions::default()
    };
    let out = transform(src, "m.ts", &opts);
    assert!(out.errors.is_empty(), "errors: {:?}", out.errors);
    let map = SourceMap::from_json_string(&out.source_map.expect("source map")).unwrap();
    Decoded {
        code: out.code,
        map,
    }
}

/// 0-based generated line index of the first line containing `needle`.
fn gen_line(code: &str, needle: &str) -> u32 {
    code.lines()
        .position(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("`{needle}` not in generated code:\n{code}")) as u32
}

/// Original 0-based line that the first token on generated line `dst_line` maps
/// to. Returns `None` if no token on that line carries a source position.
fn orig_line_of(map: &SourceMap, dst_line: u32) -> Option<u32> {
    map.get_tokens()
        .filter(|t| t.get_dst_line() == dst_line)
        .map(|t| t.get_src_line())
        .min()
}

#[test]
fn references_and_requires_map_back_to_their_original_lines() {
    // line 0: import moment from 'moment';
    // line 1: import { other } from './other';
    // line 2: (blank)
    // line 3: export function boom() {
    // line 4:   return moment(other()).nope();
    // line 5: }
    let src = "import moment from 'moment';\nimport { other } from './other';\n\nexport function boom() {\n  return moment(other()).nope();\n}\n";
    let Decoded { code, map } = cjs_with_map(src);

    // The require() statements map back to their original `import` lines (so a
    // dependency that throws on load reports the right frame) — was null before.
    let req_moment = gen_line(&code, r#"require("moment")"#);
    assert_eq!(
        orig_line_of(&map, req_moment),
        Some(0),
        "require(\"moment\") should map to the import on line 0\n{code}"
    );
    let req_other = gen_line(&code, r#"require("./other")"#);
    assert_eq!(
        orig_line_of(&map, req_other),
        Some(1),
        "require(\"./other\") should map to the import on line 1\n{code}"
    );

    // The rewritten call `moment()` → `(0, moment_1.default)()` keeps the
    // original `return moment()` line (4) — was null/0 before member_at.
    let call_line = gen_line(&code, "moment_1.default");
    assert_eq!(
        orig_line_of(&map, call_line),
        Some(4),
        "the rewritten moment() call should map to line 4\n{code}"
    );

    // The exported function declaration keeps its original line (3).
    let fn_line = gen_line(&code, "function boom");
    assert_eq!(
        orig_line_of(&map, fn_line),
        Some(3),
        "function boom should map to line 3\n{code}"
    );
}

#[test]
fn prelude_does_not_shift_mapped_lines() {
    // A default import forces the multi-line `__importDefault` prelude. If the
    // map were not offset, the body would resolve several lines too high.
    let src = "import d from './m';\nconst x = 1;\nexport const y = d() + x;\n";
    let Decoded { code, map } = cjs_with_map(src);

    let x_line = gen_line(&code, "const x = 1");
    assert_eq!(
        orig_line_of(&map, x_line),
        Some(1),
        "`const x = 1` must map to original line 1 despite the prelude\n{code}"
    );
}

#[test]
fn dynamic_import_maps_to_the_original_import_line() {
    // 0: export async function load() {
    // 1:   return import('./dep');
    // 2: }
    let src = "export async function load() {\n  return import('./dep');\n}\n";
    let Decoded { code, map } = cjs_with_map(src);
    // The whole `Promise.resolve().then(() => __importStar(require("./dep")))`
    // wrapper carries the original `import()` span, so it maps back to line 1.
    let line = gen_line(&code, r#"require("./dep")"#);
    assert_eq!(
        orig_line_of(&map, line),
        Some(1),
        "dynamic import should map to the original import on line 1\n{code}"
    );
}

#[test]
fn hoisted_jest_mock_maps_to_its_original_line() {
    // 0: import { foo } from './foo';
    // 1: const x = 1;
    // 2: jest.mock('./foo');
    // 3: foo();
    let src = "import { foo } from './foo';\nconst x = 1;\njest.mock('./foo');\nfoo();\n";
    let opts = TransformOptions {
        module: ModuleKind::CommonJs,
        target: "es2022".to_string(),
        jit_transforms: false,
        hoist_jest_mock: true,
        source_map: true,
        ..TransformOptions::default()
    };
    let out = transform(src, "m.ts", &opts);
    assert!(out.errors.is_empty(), "errors: {:?}", out.errors);
    let map = SourceMap::from_json_string(&out.source_map.expect("source map")).unwrap();
    // The mock moved up to the top of the body, but still maps to its original
    // line 2 (hoisting reorders statements without rewriting their spans).
    let line = gen_line(&out.code, "jest.mock");
    assert_eq!(
        orig_line_of(&map, line),
        Some(2),
        "hoisted jest.mock should still map to its original line 2\n{}",
        out.code
    );
}

#[test]
fn routed_exported_reference_maps_to_its_original_line() {
    // R18: a sibling reference to a directly-exported `const` is rewritten to
    // `(0, exports.a)(…)` — it must keep the original reference line, not 0.
    // 0: export const a = () => 1;
    // 1: export const b = () => a();
    let Decoded { code, map } =
        cjs_with_map("export const a = () => 1;\nexport const b = () => a();\n");
    let line = gen_line(&code, "(0, exports.a)");
    assert_eq!(
        orig_line_of(&map, line),
        Some(1),
        "the routed `exports.a` reference should map to line 1\n{code}"
    );
}

#[test]
fn routed_imported_write_maps_to_its_original_line() {
    // R19: a write to an imported binding becomes `m_1.x = …`; it must keep the
    // original assignment line.
    // 0: import { x } from './m';
    // 1: x = 1;
    let Decoded { code, map } =
        cjs_with_map("import { x } from './m';\nx = 1;\nexport const y = x;\n");
    let line = gen_line(&code, "m_1.x = 1");
    assert_eq!(
        orig_line_of(&map, line),
        Some(1),
        "the routed import write should map to line 1\n{code}"
    );
}

#[test]
fn routed_exported_let_write_maps_to_its_original_line() {
    // R15: a write to a directly-exported `let` becomes `exports.n = …` and keeps
    // its original line.
    // 0: export let n = 0;
    // 1: n = 1;
    let Decoded { code, map } = cjs_with_map("export let n = 0;\nn = 1;\n");
    let line = gen_line(&code, "exports.n = 1");
    assert_eq!(
        orig_line_of(&map, line),
        Some(1),
        "the routed exported-let write should map to line 1\n{code}"
    );
}

#[test]
fn coverage_preamble_does_not_shift_mapped_lines() {
    // Coverage instrumentation inserts counters and prepends a `var cov_… = …;`
    // preamble; the source map must still resolve a user statement to its original
    // line (the prepended preamble is offset out).
    // 0: export function f(a) {
    // 1:   return a + 1;
    // 2: }
    let opts = TransformOptions {
        module: ModuleKind::CommonJs,
        target: "es2022".to_string(),
        jit_transforms: false,
        coverage: true,
        source_map: true,
        ..TransformOptions::default()
    };
    let out = transform(
        "export function f(a) {\n  return a + 1;\n}\n",
        "m.ts",
        &opts,
    );
    assert!(out.errors.is_empty(), "errors: {:?}", out.errors);
    assert!(out.coverage_map.is_some(), "coverage map present");
    let map = SourceMap::from_json_string(&out.source_map.expect("source map")).unwrap();
    let line = gen_line(&out.code, "return a + 1");
    assert_eq!(
        orig_line_of(&map, line),
        Some(1),
        "`return a + 1` must map to original line 1 despite the coverage preamble\n{}",
        out.code
    );
}
