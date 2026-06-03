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
    assert!(
        code.contains("ns.foo"),
        "named import uses the namespace var: {code}"
    );
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
    assert!(
        code.contains("configurable: true"),
        "re-export getter is configurable: {code}"
    );
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

// --- R15: live bindings for exported mutable (`let`/`var`) bindings ----------

#[test]
fn exported_let_write_routes_through_exports() {
    // A directly-exported `let` routes every reference through `exports.<name>`
    // (tsc model), so a later write is `exports.link = v` and importers see it live.
    let code = cjs("export let link;\nexport function setLink(v) { link = v; }\n");
    assert!(code.contains("exports.link = v;"), "{code}");
}

#[test]
fn exported_let_shadowing_local_is_not_rewritten() {
    // An inner binding (here a param) that shadows the exported name has its own
    // SymbolId, so its write must stay a bare local — never `exports.link`.
    let code =
        cjs("export let link = 1;\nexport function shadow(link) { link = 99; return link; }\n");
    assert!(code.contains("link = 99"), "param write kept bare: {code}");
    assert!(
        !code.contains("exports.link = 99"),
        "shadowed write leaked to exports: {code}"
    );
}

#[test]
fn exported_const_reads_route_through_exports() {
    // R18: a directly-exported `const` routes its reads through `exports.k` so an
    // intra-module `jest.spyOn(ns, 'k')` is observed. The declaration keeps the
    // local + `exports.k = k`; sibling references become `exports.k`.
    let code = cjs("export const k = 1;\nexport function f() { return k; }\n");
    assert!(code.contains("exports.k = k;"), "{code}");
    assert!(
        code.contains("return exports.k;"),
        "sibling read routed: {code}"
    );
}

#[test]
fn exported_let_compound_and_update_assignments() {
    // Compound `+=` and prefix/postfix `++` all route through `exports.count`.
    let code = cjs(
        "export let count = 0;\nexport function add(n) { count += n; }\nexport function inc() { return ++count; }\nexport function post() { return count++; }\n",
    );
    assert!(code.contains("exports.count += n"), "{code}");
    assert!(code.contains("++exports.count"), "{code}");
    assert!(code.contains("exports.count++"), "{code}");
}

#[test]
fn export_specifier_of_local_let_is_live() {
    // `export { a }` where `a` is a module-scope `let` — writes to `a` mirror to
    // `exports.a` (here the exported name equals the local name).
    let code = cjs("let a = 0;\nexport { a };\nexport function set(v) { a = v; }\n");
    assert!(code.contains("exports.a = a = v"), "{code}");
}

#[test]
fn export_specifier_alias_of_local_let_is_live() {
    // `export { a as b }` — the write to local `a` must mirror to the EXPORTED
    // name `exports.b`, not `exports.a`.
    let code = cjs("let a = 0;\nexport { a as b };\nexport function set(v) { a = v; }\n");
    assert!(code.contains("exports.b = a = v"), "{code}");
}

// --- R18: intra-module references to exported bindings route through `exports` --

#[test]
fn intra_module_call_to_export_const_routes_through_exports() {
    // R18: a sibling call to a directly-exported `const` must go through
    // `(0, exports.fn)(…)` so `jest.spyOn(ns, 'fn')` intercepts it (it swaps
    // `exports.fn`). A bare local call would bypass the spy.
    let code = cjs(concat!(
        "export const getPastGroupDate = (d) => 'past:' + d;\n",
        "export const getGroup = (d) => getPastGroupDate(d);\n",
    ));
    assert!(
        code.contains("(0, exports.getPastGroupDate)(d)"),
        "sibling call routed through exports: {code}"
    );
}

#[test]
fn exported_function_and_class_keep_bare_intra_module_refs() {
    // tsc does NOT route function/class declarations (they're hoisted, referenced
    // by local name + `exports.x = x`). Only `export const/let/var` route.
    let fn_code = cjs("export function f(){ return 1; }\nexport const g = () => f();");
    assert!(
        fn_code.contains("=> f()"),
        "function ref stays bare: {fn_code}"
    );
    let cls = cjs("export class C {}\nexport const make = () => new C();");
    assert!(cls.contains("new C()"), "class ref stays bare: {cls}");
}

#[test]
fn destructuring_writes_to_exported_let_route_through_exports() {
    // Array, object-shorthand (with default), and renamed destructuring leaves all
    // route to `exports.<name>` (closes the prior R15 destructuring gap).
    let code = cjs(concat!(
        "export let a = 0, b = 0;\n",
        "[a, b] = [1, 2];\n",
        "({ a } = { a: 9 });\n",
        "({ x: b } = { x: 7 });\n",
    ));
    assert!(
        code.contains("[exports.a, exports.b] = [1, 2]"),
        "array: {code}"
    );
    assert!(
        code.contains("({a: exports.a} = { a: 9 })"),
        "shorthand: {code}"
    );
    assert!(
        code.contains("({x: exports.b} = { x: 7 })"),
        "renamed: {code}"
    );
}

// --- R19: writes to imported bindings route through the import namespace --------

#[test]
fn write_to_imported_binding_routes_through_namespace() {
    // R19: a spec stubbing an imported value (`isSharingApp = false`) must become
    // `flag_1.isSharingApp = false` — a bare assignment is undeclared (ReferenceError
    // under "use strict") and never updates the namespace. Reads were already routed.
    let code = cjs(concat!(
        "import { isSharingApp } from './flag';\n",
        "isSharingApp = false;\n",
        "export const x = isSharingApp;\n",
    ));
    assert!(
        code.contains("flag_1.isSharingApp = false"),
        "import write routed: {code}"
    );
    assert!(
        code.contains("flag_1.isSharingApp"),
        "read still routed: {code}"
    );
    assert!(
        !code.contains("\nisSharingApp = false"),
        "no bare undeclared write: {code}"
    );
}

#[test]
fn anonymous_default_class_gets_a_synthesized_name() {
    // `export default class {}` — a nameless class DECLARATION is a SyntaxError, and
    // `exports.default = undefined` loses the value. Match tsc: `class default_1 {}`
    // + `exports.default = default_1`.
    let code = cjs("export default class { m() { return 1; } }\n");
    assert!(
        code.contains("class default_1"),
        "anonymous default class must be given a name: {code}"
    );
    assert!(code.contains("exports.default = default_1"), "{code}");
    assert!(
        !code.contains("exports.default = undefined"),
        "the class value must not be lost: {code}"
    );
}

#[test]
fn anonymous_default_function_gets_a_synthesized_name() {
    let code = cjs("export default function () { return 1; }\n");
    assert!(
        code.contains("function default_1"),
        "anonymous default function must be given a name: {code}"
    );
    assert!(code.contains("exports.default = default_1"), "{code}");
}

#[test]
fn named_default_class_keeps_its_name() {
    let code = cjs("export default class Foo {}\n");
    assert!(code.contains("class Foo"), "{code}");
    assert!(code.contains("exports.default = Foo"), "{code}");
    assert!(
        !code.contains("default_1"),
        "named default must not be renamed: {code}"
    );
}
