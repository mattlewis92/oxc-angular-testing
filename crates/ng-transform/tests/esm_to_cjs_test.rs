//! ESM → CommonJS transform, matching TypeScript's `module: commonjs` +
//! `esModuleInterop: true` emit (verified against `tsc`).

use ng_transform::{ModuleKind, TransformOptions, transform};

fn cjs(src: &str) -> String {
    let opts = TransformOptions {
        module: ModuleKind::CommonJs,
        target: "es2022".to_string(),
        jit_transforms: false,
        ..TransformOptions::default()
    };
    let out = transform(src, "m.ts", &opts);
    assert!(out.errors.is_empty(), "errors: {:?}", out.errors);
    out.code
}

#[test]
fn dynamic_import_downlevels_to_promise_require_importstar() {
    // `import('./m')` → `Promise.resolve().then(() => __importStar(require('./m')))`,
    // matching tsc `module: commonjs` + `esModuleInterop`.
    let code = cjs("export async function load() {\n  return import('./m');\n}\n");
    assert!(
        code.contains(r#"Promise.resolve().then(() => __importStar(require("./m")))"#),
        "{code}"
    );
    // The `__importStar` helper must be emitted in the prelude.
    assert!(code.contains("var __importStar"), "{code}");
}

#[test]
fn dynamic_import_works_without_static_imports() {
    // A dynamic import with no static import/export still triggers the rewrite
    // (and the helper) — the rewriter must run unconditionally.
    let code = cjs("const m = import('./m');\n");
    assert!(
        code.contains(r#"Promise.resolve().then(() => __importStar(require("./m")))"#),
        "{code}"
    );
    assert!(code.contains("var __importStar"), "{code}");
}

#[test]
fn dynamic_import_keeps_computed_specifier() {
    // A non-literal specifier is passed through verbatim: `import(p)` → `require(p)`.
    let code = cjs("export const f = (p) => import(p);\n");
    assert!(code.contains("require(p)"), "{code}");
    assert!(
        !code.contains("require(\"p\")"),
        "must not stringify: {code}"
    );
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
fn default_via_named_specifier_uses_interop() {
    // `import { default as moment } from 'm'` is a default import (common with
    // moment-timezone). Must use __importDefault, not a bare `m_1.default`.
    let code = cjs("import { default as moment } from 'm';\nmoment();");
    assert!(code.contains("__importDefault"), "{code}");
    assert!(
        code.contains(r#"const m_1 = __importDefault(require("m"))"#),
        "{code}"
    );
    assert!(code.contains("(0, m_1.default)()"), "{code}");
}

#[test]
fn mixed_default_and_named_uses_import_star() {
    // default + named together → __importStar (the namespace is needed for both),
    // matching tsc — __importDefault would lack the named members.
    let code = cjs("import def, { named } from 'm';\ndef();\nnamed();");
    assert!(
        code.contains(r#"const m_1 = __importStar(require("m"))"#),
        "{code}"
    );
    assert!(code.contains("(0, m_1.default)()"), "{code}");
    assert!(code.contains("(0, m_1.named)()"), "{code}");
    assert!(
        !code.contains("__importDefault"),
        "should be star, not default: {code}"
    );
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
fn export_destructuring_assigns_all_bindings() {
    // Regression: ngrx `export const { selectUser, ... } = createFeature(...)`
    // — every destructured binding (object, array, rename, rest) must be exported.
    let code = cjs(
        "export const { selectUser, b: renamed } = createFeature();\nexport const [first, , third, ...rest] = arr;\n",
    );
    for name in ["selectUser", "renamed", "first", "third", "rest"] {
        assert!(
            code.contains(&format!("exports.{name} = {name}")),
            "missing export of `{name}`:\n{code}"
        );
    }
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

#[test]
fn param_shadowing_import_called_uses_the_param_not_the_import() {
    // R6: a CALL to a name that a function PARAMETER shadows must reference the
    // parameter, not the import — even when the same import is also called freely
    // elsewhere. (Hits `@angular/core`'s `keyValueArraySet` parameter; rewriting
    // the shadow to the unguarded import added empty class names → `classList.add('')`.)
    let code = cjs(concat!(
        "import { keyValueArraySet } from './dep';\n",
        "export function useImport(a, b) { keyValueArraySet(a, b, 1); }\n",
        "export function toMap(keyValueArraySet, value) { keyValueArraySet(value, 0, true); }\n",
    ));
    // Free reference → the import.
    assert!(
        code.contains("(0, dep_1.keyValueArraySet)(a, b, 1)"),
        "free import call rewritten: {code}"
    );
    // Shadowed-by-param reference → the parameter, untouched.
    assert!(
        code.contains("keyValueArraySet(value, 0, true)")
            && !code.contains("dep_1.keyValueArraySet)(value"),
        "param call must NOT be rewritten to the import: {code}"
    );
}

#[test]
fn already_commonjs_module_is_not_re_marked() {
    // R5: a module that is already CommonJS (no `import`/`export` syntax) and sets
    // its own `exports.__esModule` must NOT get a second, non-writable
    // `Object.defineProperty(exports, "__esModule", …)` prepended (which would make
    // the original assignment throw in strict mode), nor a duplicate `"use strict"`.
    let opts = TransformOptions {
        module: ModuleKind::CommonJs,
        target: "es2022".to_string(),
        jit_transforms: false,
        ..TransformOptions::default()
    };
    let code = transform(
        "\"use strict\";\nexports.__esModule = true;\nexports.foo = 42;\n",
        "index.js",
        &opts,
    )
    .code;
    assert!(
        !code.contains("Object.defineProperty(exports, \"__esModule\""),
        "no injected marker for an already-CJS module: {code}"
    );
    assert_eq!(
        code.matches("\"use strict\"").count(),
        1,
        "no duplicate use-strict directive: {code}"
    );
    // The module's own export assignments are preserved verbatim.
    assert!(code.contains("exports.__esModule = true"), "{code}");
    assert!(code.contains("exports.foo = 42"), "{code}");
}

#[test]
fn namespace_import_after_named_for_same_specifier_is_bound() {
    // R14: when a named import precedes a namespace import for the SAME specifier,
    // the namespace binding must still be materialized (it was previously folded
    // away → `ns` left undeclared → ReferenceError at runtime). The two share one
    // `__importStar` binding under the namespace var (matching tsc).
    let code = cjs(concat!(
        "import { foo } from './m';\n",
        "import * as ns from './m';\n",
        "export const a = foo;\n",
        "export const b = () => ns.bar();\n",
    ));
    assert!(
        code.contains(r#"const ns = __importStar(require("./m"))"#),
        "namespace binding emitted: {code}"
    );
    // The named import now resolves through the same namespace var.
    assert!(code.contains("ns.foo"), "named import uses the namespace var: {code}");
    assert!(code.contains("ns.bar()"), "{code}");
    // Exactly one require for the shared specifier.
    assert_eq!(code.matches(r#"require("./m")"#).count(), 1, "{code}");
}

#[test]
fn barrel_reexport_descriptor_is_configurable() {
    // R17: `export { x } from '...'` re-export getters must be `configurable: true`
    // so an `import * as ns` of the barrel yields a namespace member jest.spyOn can
    // redefine (extends the R12 fix to the re-export path).
    let code = cjs("export { foo } from './m';\n");
    assert!(
        code.contains(r#"Object.defineProperty(exports, "foo""#),
        "{code}"
    );
    assert!(code.contains("configurable: true"), "re-export getter is configurable: {code}");
}

#[test]
fn real_esm_module_still_gets_the_marker() {
    // Guard the other side of the R5 gate: a genuine ES module is still marked.
    let code = cjs("export const x = 1;\n");
    assert!(
        code.contains("Object.defineProperty(exports, \"__esModule\", { value: true })"),
        "{code}"
    );
}
