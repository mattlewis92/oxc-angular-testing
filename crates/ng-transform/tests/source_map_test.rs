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
