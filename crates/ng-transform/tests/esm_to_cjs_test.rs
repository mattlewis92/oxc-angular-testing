//! ESM → CommonJS transform, matching TypeScript's `module: commonjs` +
//! `esModuleInterop: true` emit (verified against `tsc`).

use ng_transform::{ImportMode, TransformOptions, transform};

fn cjs(src: &str) -> String {
    let opts = TransformOptions {
        import_mode: ImportMode::Require,
        esm: false,
        target: "es2022".to_string(),
        jit_transforms: false,
        ..TransformOptions::default()
    };
    let out = transform(src, "m.ts", &opts);
    assert!(out.errors.is_empty(), "errors: {:?}", out.errors);
    out.code
}

#[test]
fn header_use_strict_and_esmodule_marker() {
    let code = cjs("export const a = 1;");
    assert!(code.starts_with("\"use strict\";"), "{code}");
    assert!(
        code.contains(r#"Object.defineProperty(exports, "__esModule", { value: true })"#),
        "{code}"
    );
}

#[test]
fn named_import_rewrites_references() {
    let code = cjs("import { a, b as c } from './m';\nconsole.log(a, c);");
    assert!(code.contains(r#"const m_1 = require("./m")"#), "{code}");
    assert!(code.contains("console.log(m_1.a, m_1.b)"), "{code}");
}

#[test]
fn default_import_uses_interop_and_call_wrapper() {
    let code = cjs("import d from './m';\nd();");
    assert!(code.contains("__importDefault"), "{code}");
    assert!(
        code.contains(r#"const m_1 = __importDefault(require("./m"))"#),
        "{code}"
    );
    assert!(code.contains("(0, m_1.default)()"), "{code}");
}

#[test]
fn namespace_import_uses_import_star() {
    let code = cjs("import * as ns from './m';\nns.x();");
    assert!(code.contains("__importStar"), "{code}");
    assert!(
        code.contains(r#"const ns = __importStar(require("./m"))"#),
        "{code}"
    );
    assert!(code.contains("ns.x()"), "{code}");
}

#[test]
fn side_effect_import_is_bare_require() {
    let code = cjs("import './m';");
    assert!(code.contains(r#"require("./m")"#), "{code}");
}

#[test]
fn export_const_assigns_exports() {
    let code = cjs("export const a = 1;");
    assert!(code.contains("const a = 1"), "{code}");
    assert!(code.contains("exports.a = a"), "{code}");
}

#[test]
fn export_function_and_class() {
    let fn_code = cjs("export function foo() {}");
    assert!(fn_code.contains("function foo()"), "{fn_code}");
    assert!(fn_code.contains("exports.foo = foo"), "{fn_code}");
    let class_code = cjs("export class C {}");
    assert!(class_code.contains("class C"), "{class_code}");
    assert!(class_code.contains("exports.C = C"), "{class_code}");
}

#[test]
fn export_default_expression() {
    let code = cjs("export default 42;");
    assert!(code.contains("exports.default = 42"), "{code}");
}

#[test]
fn export_named_locals() {
    let code = cjs("const x = 1; export { x, x as y };");
    assert!(code.contains("exports.x = x"), "{code}");
    assert!(code.contains("exports.y = x"), "{code}");
}

#[test]
fn reexport_from_source() {
    let code = cjs("export { a, b as c } from './m';");
    assert!(code.contains(r#"const m_1 = require("./m")"#), "{code}");
    // Lazy getters (circular-safe), matching TypeScript's `export … from`.
    assert!(
        code.contains(r#"Object.defineProperty(exports, "a""#),
        "{code}"
    );
    assert!(code.contains("get: () => m_1.a"), "{code}");
    assert!(
        code.contains(r#"Object.defineProperty(exports, "c""#),
        "{code}"
    );
    assert!(code.contains("get: () => m_1.b"), "{code}");
}

#[test]
fn export_star_uses_export_star_helper() {
    let code = cjs("export * from './m';");
    assert!(code.contains("__exportStar"), "{code}");
    assert!(
        code.contains(r#"__exportStar(require("./m"), exports)"#),
        "{code}"
    );
}

#[test]
fn reexport_of_imported_binding_uses_namespace() {
    // `import { X } from './m'; export { X };` — the import is rewritten to
    // `m_1`, so the re-export must reference `m_1.X`, not a bare (undefined) `X`.
    // Regression: @angular/core re-exports imported bindings like REACTIVE_NODE.
    let code = cjs("import { X } from './m';\nexport { X };\nconst y = { ...X };\n");
    // Lazy getter through the namespace — circular-safe, never a bare `X`.
    assert!(
        code.contains(r#"Object.defineProperty(exports, "X""#),
        "{code}"
    );
    assert!(code.contains("get: () => m_1.X"), "{code}");
    assert!(
        !code.contains("exports.X = X;"),
        "bare re-export is undefined: {code}"
    );
    assert!(code.contains("...m_1.X"), "{code}");
}

#[test]
fn same_source_imported_and_reexported_requires_once() {
    // `import {helper} from './h'` + `export {helper} from './h'` must emit the
    // `const h_1 = require("./h")` exactly once (no duplicate declaration).
    let code = cjs("import { helper } from './h';\nexport { helper } from './h';\nhelper();");
    let count = code.matches("require(\"./h\")").count();
    assert_eq!(count, 1, "require should appear once:\n{code}");
    assert!(code.contains("get: () => h_1.helper"), "{code}");
}

#[test]
fn shadowed_import_name_is_not_rewritten() {
    // A local `a` shadowing the import must not become `m_1.a`.
    let code = cjs("import { a } from './m';\nfunction f() { const a = 5; return a; }\na();");
    assert!(code.contains("const a = 5"), "inner local kept: {code}");
    assert!(
        code.contains("(0, m_1.a)()"),
        "outer call rewritten: {code}"
    );
}
