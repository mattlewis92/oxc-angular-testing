//! ESM → CommonJS transform matching TypeScript's emit with
//! `module: "commonjs"` + `esModuleInterop: true`.
//!
//! Produced shape (verified against `tsc`):
//! - `"use strict";` + `Object.defineProperty(exports, "__esModule", { value: true });`
//! - `import { a, b as c } from "./m"` → `const m_1 = require("./m");`, refs `a`→`m_1.a`, `c`→`m_1.b`
//! - `import d from "./m"` → `const m_1 = __importDefault(require("./m"));`, `d()` → `(0, m_1.default)()`
//! - `import * as ns from "./m"` → `const ns = __importStar(require("./m"));` (refs unchanged)
//! - `import "./m"` → `require("./m");`
//! - `export const a = …` → `const a = …; exports.a = a;`
//! - `export function f(){}` / `export class C{}` → decl + `exports.f = f;`
//! - `export default E` → `exports.default = E;`
//! - `export { x, x as y }` → `exports.x = x; exports.y = x;`
//! - `export { a, b as c } from "./m"` → `const m_1 = require("./m"); exports.a = m_1.a; exports.c = m_1.b;`
//! - `export * from "./m"` → `__exportStar(require("./m"), exports);`
//!
//! The `__importDefault` / `__importStar` / `__exportStar` (+ `__createBinding` /
//! `__setModuleDefault`) helpers are injected verbatim from TypeScript's own
//! source (parsed and spliced) when used.

use std::collections::HashMap;

use oxc_allocator::Allocator;
use oxc_ast::AstBuilder;
use oxc_ast::NONE;
use oxc_ast::ast::{
    Argument, AssignmentOperator, AssignmentTarget, AssignmentTargetMaybeDefault,
    AssignmentTargetProperty, BindingPattern, Expression, FormalParameterKind,
    ImportDeclarationSpecifier, Program, PropertyKey, PropertyKind, SimpleAssignmentTarget,
    Statement, UpdateOperator, VariableDeclarationKind,
};
use oxc_semantic::SemanticBuilder;
use oxc_span::{GetSpan, SPAN};
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{Traverse, TraverseCtx, traverse_mut};

const IMPORT_DEFAULT: &str = "var __importDefault = (this && this.__importDefault) || function (mod) {\n    return (mod && mod.__esModule) ? mod : { \"default\": mod };\n};\n";

// `__createBinding`'s getter shim is `configurable: true` + settable — a
// deliberate deviation from tsc's verbatim (non-configurable getter-only) helper.
// It lets `jest.spyOn(ns, member)` redefine a namespace member from an
// `import * as ns from 'cjs-dep'` (tsc's shape throws "Cannot redefine property";
// ts-jest's namespaces happened to be spyable). Read-through via the getter is
// unchanged; the setter writes back to the source module.
//
// Both `__importStar` and `__exportStar` reference `__createBinding`, so it is
// factored out here and emitted once (tsc dedups its helpers the same way) when
// either star helper is needed; the star constants below assume it precedes them.
const CREATE_BINDING: &str = r#"var __createBinding = (this && this.__createBinding) || (Object.create ? (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    var desc = Object.getOwnPropertyDescriptor(m, k);
    if (!desc || ("get" in desc ? !m.__esModule : desc.writable || desc.configurable)) {
      desc = { enumerable: true, configurable: true, get: function() { return m[k]; }, set: function(v) { m[k] = v; } };
    }
    Object.defineProperty(o, k2, desc);
}) : (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    o[k2] = m[k];
}));
"#;

const IMPORT_STAR: &str = r#"var __setModuleDefault = (this && this.__setModuleDefault) || (Object.create ? (function(o, v) {
    Object.defineProperty(o, "default", { enumerable: true, value: v });
}) : function(o, v) {
    o["default"] = v;
});
var __importStar = (this && this.__importStar) || (function () {
    var ownKeys = function(o) {
        ownKeys = Object.getOwnPropertyNames || function (o) {
            var ar = [];
            for (var k in o) if (Object.prototype.hasOwnProperty.call(o, k)) ar[ar.length] = k;
            return ar;
        };
        return ownKeys(o);
    };
    return function (mod) {
        if (mod && mod.__esModule) return mod;
        var result = {};
        if (mod != null) for (var k = ownKeys(mod), i = 0; i < k.length; i++) if (k[i] !== "default") __createBinding(result, mod, k[i]);
        __setModuleDefault(result, mod);
        return result;
    };
})();
"#;

const EXPORT_STAR: &str = r#"var __exportStar = (this && this.__exportStar) || function(m, exports) {
    for (var p in m) if (p !== "default" && !Object.prototype.hasOwnProperty.call(exports, p)) __createBinding(exports, m, p);
};
"#;

/// Replacement for an imported binding reference: `<ns_var>.<member>`.
#[derive(Clone)]
struct Replacement {
    ns_var: String,
    member: String,
}

#[derive(Default)]
struct HelperNeeds {
    import_default: bool,
    import_star: bool,
    export_star: bool,
}

/// Run the ESM → CJS transform on `program` (already TS-stripped / lowered).
///
/// Returns a **prelude string** of interop helper definitions (`__importDefault`
/// etc.) the caller must place after `"use strict";` and before the generated
/// code. Helpers are emitted as text rather than spliced AST so their (foreign)
/// spans never reach the codegen source-map builder.
#[must_use]
/// How a module specifier's `require(...)` is wrapped, aggregated across every
/// import statement for that source (tsc's `esModuleInterop` rule).
enum ImportKind {
    /// `__importStar(require(...))` — any namespace import, or mixed default+named.
    Star,
    /// `__importDefault(require(...))` — default import(s) only.
    Default,
    /// `require(...)` — named import(s) only.
    Plain,
}

/// Result of the ESM→CJS rewrite.
pub struct CjsResult {
    /// Interop helper text the caller prepends after `"use strict";`.
    pub prelude: String,
    /// Whether the module had ES module syntax (`import`/`export`) and was
    /// therefore converted — i.e. the `__esModule` marker was emitted and the
    /// caller should add the `"use strict";` directive. False for a module that
    /// is already CommonJS (only `exports.x = …` / `require(…)`, no `import`/
    /// `export`): we leave it untouched (no marker, no extra directive) so we
    /// never re-mark a module that already sets its own `exports.__esModule`,
    /// matching tsc, which only marks genuine external modules.
    pub converted: bool,
}

pub fn esm_to_cjs<'a>(allocator: &'a Allocator, program: &mut Program<'a>) -> CjsResult {
    let ast = AstBuilder::new(allocator);

    // A module is "already CommonJS" when it has no ESM import/export syntax. Such
    // a file is not run through interop marking (see `CjsResult::converted`); we
    // still walk it below to rewrite any dynamic `import()` → `require()`.
    let has_esm_syntax = program.body.iter().any(|s| {
        matches!(
            s,
            Statement::ImportDeclaration(_)
                | Statement::ExportNamedDeclaration(_)
                | Statement::ExportDefaultDeclaration(_)
                | Statement::ExportAllDeclaration(_)
        )
    });

    // 1. Aggregate every import statement per source, then pick a canonical var +
    //    import kind per source. A module may be imported by several statements; the
    //    canonical var prefers a namespace local, so `import { x }` + `import * as ns`
    //    of the SAME module share one `__importStar` binding (matching tsc) and the
    //    namespace local is always declared — regardless of statement order.
    //    (Previously a namespace import that FOLLOWED a named import for the same
    //    specifier was folded away, its local left undeclared → `ReferenceError`.)
    let mut module_vars: HashMap<String, String> = HashMap::new();
    let mut used_names: HashMap<String, u32> = HashMap::new();
    let mut replacements: HashMap<String, Replacement> = HashMap::new();
    let mut needs = HelperNeeds::default();

    #[derive(Default)]
    struct SourceAgg {
        ns_local: Option<String>,
        has_default: bool,
        has_named: bool,
        has_namespace: bool,
    }
    let mut order: Vec<String> = Vec::new();
    let mut agg: HashMap<String, SourceAgg> = HashMap::new();
    for stmt in &program.body {
        let Statement::ImportDeclaration(import) = stmt else {
            continue;
        };
        let Some(specifiers) = &import.specifiers else {
            continue; // side-effect only
        };
        let source = import.source.value.as_str().to_string();
        if !agg.contains_key(&source) {
            order.push(source.clone());
        }
        let e = agg.entry(source).or_default();
        for spec in specifiers {
            match spec {
                ImportDeclarationSpecifier::ImportSpecifier(s) => {
                    if s.imported.name().as_str() == "default" {
                        e.has_default = true;
                    } else {
                        e.has_named = true;
                    }
                }
                ImportDeclarationSpecifier::ImportDefaultSpecifier(_) => e.has_default = true,
                ImportDeclarationSpecifier::ImportNamespaceSpecifier(ns) => {
                    e.has_namespace = true;
                    e.ns_local = Some(ns.local.name.as_str().to_string());
                }
            }
        }
    }
    // Canonical var + kind per source, in first-appearance order (deterministic
    // var numbering). A namespace local wins the var; otherwise a generated name.
    let mut import_kind: HashMap<String, ImportKind> = HashMap::new();
    for source in &order {
        let e = &agg[source];
        let var = e
            .ns_local
            .clone()
            .unwrap_or_else(|| unique_module_var(source, &mut used_names));
        module_vars.insert(source.clone(), var);
        let kind = if e.has_namespace || (e.has_default && e.has_named) {
            ImportKind::Star
        } else if e.has_default {
            ImportKind::Default
        } else {
            ImportKind::Plain
        };
        import_kind.insert(source.clone(), kind);
    }
    // A replacement per named/default local → `<canonical var>.<member>`.
    for stmt in &program.body {
        let Statement::ImportDeclaration(import) = stmt else {
            continue;
        };
        let Some(specifiers) = &import.specifiers else {
            continue;
        };
        let ns_var = module_vars[import.source.value.as_str()].clone();
        for spec in specifiers {
            match spec {
                ImportDeclarationSpecifier::ImportSpecifier(s) => {
                    replacements.insert(
                        s.local.name.as_str().to_string(),
                        Replacement {
                            ns_var: ns_var.clone(),
                            member: s.imported.name().as_str().to_string(),
                        },
                    );
                }
                ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => {
                    replacements.insert(
                        s.local.name.as_str().to_string(),
                        Replacement {
                            ns_var: ns_var.clone(),
                            member: "default".to_string(),
                        },
                    );
                }
                ImportDeclarationSpecifier::ImportNamespaceSpecifier(_) => {}
            }
        }
    }

    // 2. Rewrite references to default/named import locals (symbol-aware) and
    //    rewrite dynamic `import()` → `require()`. Always runs: a dynamic import
    //    can appear with no static imports (so `replacements` is empty).
    {
        let scoping = SemanticBuilder::new()
            .build(program)
            .semantic
            .into_scoping();
        let mut symbol_map = build_symbol_map(program, &replacements);
        // Directly-exported `const`/`let`/`var` route every reference through
        // `exports.<name>` (tsc model): so `jest.spyOn(ns, name)` intercepts
        // intra-module calls (R18) and mutable exports are live (R15). Added to
        // `symbol_map` with the `exports` namespace alongside imports.
        for (sym, name) in build_directly_exported_map(program) {
            symbol_map.insert(
                sym,
                Replacement {
                    ns_var: "exports".to_string(),
                    member: name,
                },
            );
        }
        // `export { local as ll }` of a module-scope `let`/`var` keeps its local
        // (tsc keeps the binding) and *mirrors* writes to `exports.ll`.
        let live_bindings = build_specifier_export_map(program);
        let mut rewriter = RefRewriter {
            symbol_map,
            live_bindings,
            import_star_used: false,
        };
        traverse_mut(&mut rewriter, allocator, program, scoping, ());
        if rewriter.import_star_used {
            needs.import_star = true;
        }
    }

    // 3. Rewrite statements: imports → require, exports → exports.x.
    let old_body = std::mem::replace(&mut program.body, ast.vec());
    let mut exported_names: Vec<String> = Vec::new(); // for the `void 0` header
    let mut body: Vec<Statement<'a>> = Vec::new();
    // Sources whose `const <var> = require(...)` has already been emitted, so a
    // module that is both imported and re-exported is required only once.
    let mut emitted_requires: std::collections::HashSet<String> = std::collections::HashSet::new();

    for stmt in old_body {
        match stmt {
            Statement::ImportDeclaration(import) => {
                rewrite_import(
                    &import,
                    &module_vars,
                    &import_kind,
                    &mut needs,
                    &mut emitted_requires,
                    ast,
                    &mut body,
                );
            }
            Statement::ExportNamedDeclaration(export) => {
                rewrite_export_named(
                    export.unbox(),
                    &mut module_vars,
                    &mut used_names,
                    &mut exported_names,
                    &mut emitted_requires,
                    &replacements,
                    ast,
                    &mut body,
                );
            }
            Statement::ExportDefaultDeclaration(export) => {
                rewrite_export_default(
                    export.unbox(),
                    &mut exported_names,
                    &mut used_names,
                    ast,
                    &mut body,
                );
            }
            Statement::ExportAllDeclaration(export) => {
                needs.export_star = true;
                // `export * from "m"` → `__exportStar(require("m"), exports);`
                // (named `export * as ns` handled as a namespace export.)
                let span = export.span;
                if let Some(name) = &export.exported {
                    // export * as ns from "m"
                    let req = require_call_at(export.source.value.as_str(), span, ast);
                    let star = call(ident("__importStar", ast), vec![req], ast);
                    needs.import_star = true;
                    body.push(assign_export_stmt(name.name().as_str(), star, ast));
                    exported_names.push(name.name().as_str().to_string());
                } else {
                    let req = require_call_at(export.source.value.as_str(), span, ast);
                    let call_expr = call(
                        ident("__exportStar", ast),
                        vec![req, ident("exports", ast)],
                        ast,
                    );
                    body.push(ast.statement_expression(span, call_expr));
                }
            }
            other => body.push(other),
        }
    }

    // 4. Header in the AST: `__esModule` marker + `exports.x = void 0;` hoist.
    //    Interop helpers are returned as a text prelude (see below). The marker is
    //    only emitted for genuine ES modules — an already-CommonJS file keeps its
    //    own exports (and its own `exports.__esModule`, if any) intact.
    let mut final_body = ast.vec();
    if has_esm_syntax {
        final_body.push(es_module_marker(ast));
    }
    if !exported_names.is_empty() {
        final_body.push(void0_hoist(&exported_names, ast));
    }
    for s in body {
        final_body.push(s);
    }
    program.body = final_body;

    // Helper prelude (verbatim TypeScript helpers), only what's used.
    let mut prelude = String::new();
    if needs.import_default {
        prelude.push_str(IMPORT_DEFAULT);
    }
    // `__createBinding` underpins both star helpers; emit it once, before either.
    if needs.import_star || needs.export_star {
        prelude.push_str(CREATE_BINDING);
    }
    if needs.import_star {
        prelude.push_str(IMPORT_STAR);
    }
    if needs.export_star {
        prelude.push_str(EXPORT_STAR);
    }
    CjsResult {
        prelude,
        converted: has_esm_syntax,
    }
}

/// Sanitize a module source into a TS-style `<base>_<n>` variable name.
fn unique_module_var(source: &str, used: &mut HashMap<String, u32>) -> String {
    let base = source.rsplit('/').next().unwrap_or(source);
    let base = base
        .strip_suffix(".js")
        .or_else(|| base.strip_suffix(".ts"))
        .unwrap_or(base);
    let mut ident: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '$' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if ident.is_empty() || ident.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        ident.insert(0, '_');
    }
    let n = used.entry(ident.clone()).or_insert(0);
    *n += 1;
    format!("{ident}_{n}")
}

/// Map each replacement's local NAME to its binding `SymbolId` so references can
/// be rewritten without tripping on shadowing.
fn build_symbol_map<'a>(
    program: &Program<'a>,
    replacements: &HashMap<String, Replacement>,
) -> HashMap<SymbolId, Replacement> {
    let mut map = HashMap::new();
    for stmt in &program.body {
        let Statement::ImportDeclaration(import) = stmt else {
            continue;
        };
        let Some(specifiers) = &import.specifiers else {
            continue;
        };
        for spec in specifiers {
            let (local, name) = match spec {
                ImportDeclarationSpecifier::ImportSpecifier(s) => (&s.local, s.local.name.as_str()),
                ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => {
                    (&s.local, s.local.name.as_str())
                }
                ImportDeclarationSpecifier::ImportNamespaceSpecifier(_) => continue,
            };
            if let (Some(repl), Some(symbol_id)) = (replacements.get(name), local.symbol_id.get()) {
                map.insert(symbol_id, repl.clone());
            }
        }
    }
    map
}

/// Directly-exported `const`/`let`/`var` bindings (`export const/let/var x = …`)
/// → exported name. tsc routes EVERY reference to these through `exports.<name>`
/// (read, call, and write), so intra-module `jest.spyOn(ns, name)` intercepts and
/// mutable exports stay live. They are added to `symbol_map` with the `exports`
/// namespace. (Function/class declarations and `export { local }` specifier
/// exports are NOT routed — tsc keeps bare local references for those.)
fn build_directly_exported_map<'a>(program: &Program<'a>) -> HashMap<SymbolId, String> {
    let mut map: HashMap<SymbolId, String> = HashMap::new();
    for stmt in &program.body {
        let Statement::ExportNamedDeclaration(export) = stmt else {
            continue;
        };
        if let Some(oxc_ast::ast::Declaration::VariableDeclaration(v)) = &export.declaration {
            let mut binders: HashMap<&str, SymbolId> = HashMap::new();
            for d in &v.declarations {
                collect_pattern_symbols(&d.id, &mut binders);
            }
            for (name, sym) in binders {
                map.insert(sym, name.to_string());
            }
        }
    }
    map
}

/// `export { local as ll }` of a module-scope `let`/`var` (NOT an import) →
/// local's `SymbolId` → exported name `ll`. tsc keeps the local binding (bare
/// reads) but MIRRORS each write into `exports.ll` (`exports.ll = local = v`) so
/// importers observe live values. (Specifier-exported `const` needs no mirror —
/// it can't be reassigned — so only `let`/`var` are collected here.)
fn build_specifier_export_map<'a>(program: &Program<'a>) -> HashMap<SymbolId, String> {
    // First collect every module-scope `let`/`var` binding name → SymbolId, so
    // the `export { … }` (alias) branch can resolve a local name to its symbol.
    let mut module_let_var: HashMap<&str, SymbolId> = HashMap::new();
    for stmt in &program.body {
        let decl = match stmt {
            Statement::VariableDeclaration(v) => Some(&**v),
            Statement::ExportNamedDeclaration(e) => match &e.declaration {
                Some(oxc_ast::ast::Declaration::VariableDeclaration(v)) => Some(&**v),
                _ => None,
            },
            _ => None,
        };
        let Some(v) = decl else { continue };
        if v.kind == VariableDeclarationKind::Const {
            continue;
        }
        for d in &v.declarations {
            collect_pattern_symbols(&d.id, &mut module_let_var);
        }
    }

    let mut map: HashMap<SymbolId, String> = HashMap::new();
    for stmt in &program.body {
        let Statement::ExportNamedDeclaration(export) = stmt else {
            continue;
        };
        // Directly-exported declarations are handled by `build_directly_exported_map`
        // (routed through `exports`, not mirrored). Only the specifier form below
        // keeps a local + mirrors writes.
        if export.declaration.is_some() {
            continue;
        }
        // `export { local as ll }` with no source — alias of a local `let`/`var`.
        if export.source.is_none() {
            for spec in &export.specifiers {
                let local = spec.local.name();
                if let Some(&sym) = module_let_var.get(local.as_str()) {
                    map.insert(sym, spec.exported.name().as_str().to_string());
                }
            }
        }
    }
    map
}

/// Collect every bound identifier in a binding pattern → its `SymbolId`,
/// recursing through destructuring, defaults, and rest.
fn collect_pattern_symbols<'a>(pattern: &BindingPattern<'a>, out: &mut HashMap<&'a str, SymbolId>) {
    match pattern {
        BindingPattern::BindingIdentifier(id) => {
            if let Some(sym) = id.symbol_id.get() {
                out.insert(id.name.as_str(), sym);
            }
        }
        BindingPattern::ObjectPattern(obj) => {
            for prop in &obj.properties {
                collect_pattern_symbols(&prop.value, out);
            }
            if let Some(rest) = &obj.rest {
                collect_pattern_symbols(&rest.argument, out);
            }
        }
        BindingPattern::ArrayPattern(arr) => {
            for elem in arr.elements.iter().flatten() {
                collect_pattern_symbols(elem, out);
            }
            if let Some(rest) = &arr.rest {
                collect_pattern_symbols(&rest.argument, out);
            }
        }
        BindingPattern::AssignmentPattern(assign) => collect_pattern_symbols(&assign.left, out),
    }
}

struct RefRewriter {
    symbol_map: HashMap<SymbolId, Replacement>,
    /// Module-scope mutable exported bindings (`SymbolId` → exported name). Writes
    /// to these are mirrored into `exports.<name>` so importers see live values.
    live_bindings: HashMap<SymbolId, String>,
    /// Set when a dynamic `import()` was rewritten (→ the `__importStar` helper
    /// is needed).
    import_star_used: bool,
}

impl<'a> Traverse<'a, ()> for RefRewriter {
    // Dynamic `import(x)` → `Promise.resolve().then(() => __importStar(require(x)))`,
    // matching tsc's `module: commonjs` + `esModuleInterop` emit (import
    // attributes are dropped, as tsc does). Done on *exit* so the traverser has
    // already walked the original children and won't descend into the
    // synthesized arrow (whose scope isn't registered → would panic).
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut TraverseCtx<'a, ()>) {
        if let Expression::ImportExpression(imp) = expr {
            let ast = ctx.ast;
            let span = imp.span;
            let source = std::mem::replace(&mut imp.source, ast.expression_null_literal(span));
            *expr = dynamic_require(source, span, ast);
            self.import_star_used = true;
            return;
        }

        // Mirror a write to a live exported binding into `exports.<name>`. Done on
        // *exit* so we wrap the already-traversed expression and don't re-descend
        // into the synthesized wrapper (which would recurse forever). The local
        // `let`/`var` is kept as the read source-of-truth; we add `exports.x = …`
        // alongside every write so importers observe the new value.
        match expr {
            // `x = v`, `x += v`, `x **= v`, … → `exports.x = (x <op>= v)`.
            // Only a bare-identifier LHS; destructuring leaves are handled in the
            // assignment-target hooks below.
            Expression::AssignmentExpression(assign) => {
                let AssignmentTarget::AssignmentTargetIdentifier(id) = &assign.left else {
                    return;
                };
                let Some(name) = self.live_binding_name(id.reference_id.get(), ctx) else {
                    return;
                };
                let name = name.to_string();
                let inner = std::mem::replace(expr, ctx.ast.expression_null_literal(SPAN));
                *expr = export_write_wrap(&name, inner, ctx.ast);
            }
            // `x++` / `++x` / `x--` / `--x` → preserve value semantics:
            //   `++x` (prefix)  → `exports.x = ++x`
            //   `x++` (postfix) → `(exports.x = ++x) - 1` (`+ 1` for `--`)
            Expression::UpdateExpression(update) => {
                let SimpleAssignmentTarget::AssignmentTargetIdentifier(id) = &update.argument
                else {
                    return;
                };
                let Some(name) = self.live_binding_name(id.reference_id.get(), ctx) else {
                    return;
                };
                let name = name.to_string();
                let was_prefix = update.prefix;
                let op = update.operator;
                update.prefix = true;
                let inner = std::mem::replace(expr, ctx.ast.expression_null_literal(SPAN));
                *expr = export_update_wrap(&name, inner, was_prefix, op, ctx.ast);
            }
            _ => {}
        }
    }

    // Route a WRITE to a namespaced binding through its namespace object:
    // `isSharingApp = v` → `flag_1.isSharingApp = v` (import write, R19) and
    // `exportedLet = v` → `exports.exportedLet = v` (directly-exported, R18). Also
    // fires for compound assignment, `++`/`--` (the update's argument is a
    // SimpleAssignmentTarget), and array/property destructuring leaves — so all
    // resolve to `<ns>.<member>`. Scope-aware via SymbolId: a shadowing local of
    // the same name resolves to a different symbol and is left untouched.
    fn enter_simple_assignment_target(
        &mut self,
        target: &mut SimpleAssignmentTarget<'a>,
        ctx: &mut TraverseCtx<'a, ()>,
    ) {
        if let SimpleAssignmentTarget::AssignmentTargetIdentifier(id) = target
            && let Some(repl) = self.reference_replacement(id.reference_id.get(), ctx)
        {
            *target = member_target(&repl.ns_var, &repl.member, id.span, ctx.ast);
        }
    }

    // Object-shorthand destructuring `({ x } = obj)` where `x` routes → rewrite to
    // `({ x: <ns>.x } = obj)` (preserving any `= default`). The shorthand binding
    // is an `IdentifierReference`, not an assignment target, so the
    // simple-assignment-target hook above can't reach it.
    fn enter_assignment_target_property(
        &mut self,
        prop: &mut AssignmentTargetProperty<'a>,
        ctx: &mut TraverseCtx<'a, ()>,
    ) {
        let AssignmentTargetProperty::AssignmentTargetPropertyIdentifier(ident) = prop else {
            return;
        };
        let Some(repl) = self.reference_replacement(ident.binding.reference_id.get(), ctx) else {
            return;
        };
        let ast = ctx.ast;
        let span = ident.binding.span;
        let key = PropertyKey::StaticIdentifier(
            ast.alloc_identifier_name(span, ast.allocator.alloc_str(ident.binding.name.as_str())),
        );
        let member: AssignmentTarget = ast
            .member_expression_static(
                span,
                ast.expression_identifier(span, ast.allocator.alloc_str(&repl.ns_var)),
                ast.identifier_name(span, ast.allocator.alloc_str(&repl.member)),
                false,
            )
            .into();
        let binding = match ident.init.take() {
            Some(init) => AssignmentTargetMaybeDefault::AssignmentTargetWithDefault(
                ast.alloc(ast.assignment_target_with_default(span, member, init)),
            ),
            None => AssignmentTargetMaybeDefault::from(member),
        };
        *prop = AssignmentTargetProperty::AssignmentTargetPropertyProperty(
            ast.alloc(ast.assignment_target_property_property(span, key, binding, false)),
        );
    }

    fn enter_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut TraverseCtx<'a, ()>) {
        // Wrap import-binding calls as `(0, ns.member)(...)` to drop `this`.
        if let Expression::CallExpression(call_expr) = expr
            && let Some(repl) = self.callee_replacement(&call_expr.callee, ctx)
        {
            let ast = ctx.ast;
            let span = call_expr.callee.span();
            let member = member_at(&repl.ns_var, &repl.member, span, ast);
            call_expr.callee = sequence_zero(member, ast);
            return;
        }
        let Expression::Identifier(id) = expr else {
            return;
        };
        let Some(repl) = self.reference_replacement(id.reference_id.get(), ctx) else {
            return;
        };
        *expr = member_at(&repl.ns_var, &repl.member, id.span, ctx.ast);
    }
}

impl RefRewriter {
    fn reference_replacement(
        &self,
        reference_id: Option<oxc_syntax::reference::ReferenceId>,
        ctx: &TraverseCtx<'_, ()>,
    ) -> Option<Replacement> {
        let rid = reference_id?;
        let symbol_id = ctx.scoping().get_reference(rid).symbol_id()?;
        self.symbol_map.get(&symbol_id).cloned()
    }

    /// Resolve a reference to its binding `SymbolId` and, if that binding is a
    /// live exported mutable binding, return its exported name. Scope-aware: an
    /// inner shadow of the same name resolves to a different `SymbolId` and is not
    /// rewritten.
    fn live_binding_name(
        &self,
        reference_id: Option<oxc_syntax::reference::ReferenceId>,
        ctx: &TraverseCtx<'_, ()>,
    ) -> Option<&str> {
        let rid = reference_id?;
        let symbol_id = ctx.scoping().get_reference(rid).symbol_id()?;
        self.live_bindings.get(&symbol_id).map(String::as_str)
    }

    fn callee_replacement(
        &self,
        callee: &Expression<'_>,
        ctx: &TraverseCtx<'_, ()>,
    ) -> Option<Replacement> {
        let Expression::Identifier(id) = callee else {
            return None;
        };
        // The reference's resolved binding is authoritative: rewrite only when it
        // is our import symbol, never when a parameter/local of the same name
        // shadows the import (else we'd miscompile the shadow — a general
        // correctness bug, e.g. `@angular/core`'s `keyValueArraySet` parameter).
        // esm_to_cjs runs a fresh `SemanticBuilder` before this traversal, so every
        // identifier (including ones synthesized by earlier passes) carries a
        // `reference_id`; there is no unresolved-by-name fallback to make.
        self.reference_replacement(id.reference_id.get(), ctx)
    }
}

// --- statement rewriters ---------------------------------------------------

fn rewrite_import<'a>(
    import: &oxc_ast::ast::ImportDeclaration<'a>,
    module_vars: &HashMap<String, String>,
    import_kind: &HashMap<String, ImportKind>,
    needs: &mut HelperNeeds,
    emitted: &mut std::collections::HashSet<String>,
    ast: AstBuilder<'a>,
    out: &mut Vec<Statement<'a>>,
) {
    let source = import.source.value.as_str();
    let span = import.span;
    if import.specifiers.is_none() {
        // side-effect import → `require("…");` (skip if already required).
        if emitted.insert(source.to_string()) {
            out.push(ast.statement_expression(span, require_call_at(source, span, ast)));
        }
        return;
    }
    let ns_var = module_vars
        .get(source)
        .cloned()
        .unwrap_or_else(|| source.to_string());
    if !emitted.insert(source.to_string()) {
        // Already required (multiple import statements / also re-exported) — every
        // binding for this source shares the one canonical var, so emit once.
        return;
    }
    let req = require_call_at(source, span, ast);
    // The interop wrapper is decided per-source (across all its import statements),
    // not per-statement — so a named + namespace import of the same module still
    // gets `__importStar`. See `ImportKind`.
    let init = match import_kind.get(source) {
        Some(ImportKind::Star) => {
            needs.import_star = true;
            call(ident("__importStar", ast), vec![req], ast)
        }
        Some(ImportKind::Default) => {
            needs.import_default = true;
            call(ident("__importDefault", ast), vec![req], ast)
        }
        _ => req,
    };
    out.push(const_decl_at(&ns_var, init, span, ast));
}

#[allow(clippy::too_many_arguments)]
fn rewrite_export_named<'a>(
    export: oxc_ast::ast::ExportNamedDeclaration<'a>,
    module_vars: &mut HashMap<String, String>,
    used_names: &mut HashMap<String, u32>,
    exported_names: &mut Vec<String>,
    emitted: &mut std::collections::HashSet<String>,
    replacements: &HashMap<String, Replacement>,
    ast: AstBuilder<'a>,
    out: &mut Vec<Statement<'a>>,
) {
    // `export { a, b as c } from "./m"` — re-export.
    if let Some(source) = &export.source {
        let src = source.value.as_str();
        let ns_var = module_vars
            .entry(src.to_string())
            .or_insert_with(|| unique_module_var(src, used_names))
            .clone();
        // Require the source once (it may also have been imported).
        if emitted.insert(src.to_string()) {
            out.push(const_decl_at(
                &ns_var,
                require_call_at(src, export.span, ast),
                export.span,
                ast,
            ));
        }
        for spec in &export.specifiers {
            let local = spec.local.name();
            let exported = spec.exported.name();
            // Lazy getter so re-exports work across circular module graphs.
            out.push(define_export_getter(
                exported.as_str(),
                member(&ns_var, local.as_str(), ast),
                ast,
            ));
        }
        return;
    }

    // `export <decl>` — keep the declaration, then assign each binding.
    if let Some(decl) = export.declaration {
        let names = declaration_binding_names(&decl);
        out.push(Statement::from(decl));
        for name in names {
            out.push(assign_export_stmt(&name, ident(&name, ast), ast));
            exported_names.push(name);
        }
        return;
    }

    // `export { x, y as z }` — assign from locals. If a local is actually an
    // imported binding (e.g. `import { X } from './m'; export { X };`), the
    // import was rewritten to `_m.X`, so re-export through the namespace rather
    // than a now-undefined bare identifier.
    for spec in &export.specifiers {
        let local = spec.local.name();
        let exported = spec.exported.name();
        match replacements.get(local.as_str()) {
            // Re-export of an imported binding → lazy getter (circular-safe).
            Some(repl) => {
                out.push(define_export_getter(
                    exported.as_str(),
                    member(&repl.ns_var, &repl.member, ast),
                    ast,
                ));
            }
            // Genuine local binding → eager assignment.
            None => {
                out.push(assign_export_stmt(
                    exported.as_str(),
                    ident(local.as_str(), ast),
                    ast,
                ));
                exported_names.push(exported.as_str().to_string());
            }
        }
    }
}

fn rewrite_export_default<'a>(
    export: oxc_ast::ast::ExportDefaultDeclaration<'a>,
    exported_names: &mut Vec<String>,
    used_names: &mut HashMap<String, u32>,
    ast: AstBuilder<'a>,
    out: &mut Vec<Statement<'a>>,
) {
    use oxc_ast::ast::ExportDefaultDeclarationKind as K;
    exported_names.push("default".to_string());
    match export.declaration {
        // An ANONYMOUS `export default function/class` must be given a name: a
        // nameless function/class *declaration* is a SyntaxError, and emitting
        // `exports.default = undefined` would lose the value. Match tsc: synthesize
        // a `default_N` binding and assign it. A named declaration keeps its name.
        K::FunctionDeclaration(mut func) => {
            let name = ensure_default_name(&mut func.id, used_names, ast);
            out.push(Statement::FunctionDeclaration(func));
            out.push(assign_export_stmt("default", ident(&name, ast), ast));
        }
        K::ClassDeclaration(mut class) => {
            let name = ensure_default_name(&mut class.id, used_names, ast);
            out.push(Statement::ClassDeclaration(class));
            out.push(assign_export_stmt("default", ident(&name, ast), ast));
        }
        expr => {
            // An expression: `export default <expr>;`
            let expression = expr.into_expression();
            out.push(assign_export_stmt("default", expression, ast));
        }
    }
}

/// Return the binding name of an `export default` function/class, synthesizing a
/// collision-safe `default_N` (via the shared name machinery) and writing it into
/// `id` when the declaration is anonymous. Mirrors tsc's `default_N` naming.
fn ensure_default_name<'a>(
    id: &mut Option<oxc_ast::ast::BindingIdentifier<'a>>,
    used_names: &mut HashMap<String, u32>,
    ast: AstBuilder<'a>,
) -> String {
    if let Some(existing) = id {
        return existing.name.as_str().to_string();
    }
    let name = unique_module_var("default", used_names);
    let arena = ast.allocator.alloc_str(&name);
    *id = Some(ast.binding_identifier(SPAN, arena));
    name
}

fn declaration_binding_names(decl: &oxc_ast::ast::Declaration<'_>) -> Vec<String> {
    use oxc_ast::ast::Declaration as D;
    match decl {
        D::VariableDeclaration(v) => {
            let mut names = Vec::new();
            for d in &v.declarations {
                collect_binding_idents(&d.id, &mut names);
            }
            names
        }
        D::FunctionDeclaration(f) => {
            f.id.as_ref()
                .map(|i| vec![i.name.as_str().to_string()])
                .unwrap_or_default()
        }
        D::ClassDeclaration(c) => {
            c.id.as_ref()
                .map(|i| vec![i.name.as_str().to_string()])
                .unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

/// Collect every bound identifier in a binding pattern, recursing through object
/// / array destructuring, defaults, and rest — so `export const { a, b } = …`
/// and `export const [x, ...y] = …` export all of `a`, `b`, `x`, `y`.
fn collect_binding_idents(pattern: &BindingPattern<'_>, out: &mut Vec<String>) {
    match pattern {
        BindingPattern::BindingIdentifier(id) => out.push(id.name.as_str().to_string()),
        BindingPattern::ObjectPattern(obj) => {
            for prop in &obj.properties {
                collect_binding_idents(&prop.value, out);
            }
            if let Some(rest) = &obj.rest {
                collect_binding_idents(&rest.argument, out);
            }
        }
        BindingPattern::ArrayPattern(arr) => {
            for elem in arr.elements.iter().flatten() {
                collect_binding_idents(elem, out);
            }
            if let Some(rest) = &arr.rest {
                collect_binding_idents(&rest.argument, out);
            }
        }
        BindingPattern::AssignmentPattern(assign) => collect_binding_idents(&assign.left, out),
    }
}

// --- small builders --------------------------------------------------------

fn ident<'a>(name: &str, ast: AstBuilder<'a>) -> Expression<'a> {
    ast.expression_identifier(SPAN, ast.allocator.alloc_str(name))
}

fn member<'a>(object: &str, property: &str, ast: AstBuilder<'a>) -> Expression<'a> {
    member_at(object, property, SPAN, ast)
}

/// Like [`member`] but stamps the rewritten `object.property` with `span` so the
/// source map points back at the original reference (e.g. the `moment` token
/// that became `moment_1.default`) instead of mapping to position 0.
fn member_at<'a>(
    object: &str,
    property: &str,
    span: oxc_span::Span,
    ast: AstBuilder<'a>,
) -> Expression<'a> {
    ast.member_expression_static(
        span,
        ast.expression_identifier(span, ast.allocator.alloc_str(object)),
        ast.identifier_name(span, ast.allocator.alloc_str(property)),
        false,
    )
    .into()
}

/// `<object>.<property>` as a [`SimpleAssignmentTarget`] (write position).
fn member_target<'a>(
    object: &str,
    property: &str,
    span: oxc_span::Span,
    ast: AstBuilder<'a>,
) -> SimpleAssignmentTarget<'a> {
    ast.member_expression_static(
        span,
        ast.expression_identifier(span, ast.allocator.alloc_str(object)),
        ast.identifier_name(span, ast.allocator.alloc_str(property)),
        false,
    )
    .into()
}

fn call<'a>(
    callee: Expression<'a>,
    args: Vec<Expression<'a>>,
    ast: AstBuilder<'a>,
) -> Expression<'a> {
    let mut a = ast.vec();
    for arg in args {
        a.push(Argument::from(arg));
    }
    ast.expression_call(SPAN, callee, NONE, a, false)
}

/// Builds `require("…")`, stamping the call with `span` so a module that throws
/// while loading reports the original `import`/`export … from` statement as the
/// stack frame, matching tsc.
fn require_call_at<'a>(source: &str, span: oxc_span::Span, ast: AstBuilder<'a>) -> Expression<'a> {
    let arg = ast.expression_string_literal(span, ast.allocator.alloc_str(source), None);
    let callee = ast.expression_identifier(span, "require");
    ast.expression_call(span, callee, NONE, ast.vec1(Argument::from(arg)), false)
}

/// Dynamic import downlevel: `import(<source>)` →
/// `Promise.resolve().then(() => __importStar(require(<source>)))`, matching tsc's
/// `module: commonjs` + `esModuleInterop` emit. The source expression is kept
/// verbatim (string literal or computed).
fn dynamic_require<'a>(
    source: Expression<'a>,
    span: oxc_span::Span,
    ast: AstBuilder<'a>,
) -> Expression<'a> {
    // All wrapper nodes carry the original `import()` span, so the whole
    // expression maps back to the source location rather than position 0.
    let call_at = |callee: Expression<'a>, arg: Option<Expression<'a>>| {
        let args = match arg {
            Some(a) => ast.vec1(Argument::from(a)),
            None => ast.vec(),
        };
        ast.expression_call(span, callee, NONE, args, false)
    };
    // require(<source>)
    let require_call = call_at(ast.expression_identifier(span, "require"), Some(source));
    // () => __importStar(require(<source>))
    let import_star = call_at(
        ast.expression_identifier(span, "__importStar"),
        Some(require_call),
    );
    let factory = arrow_getter(import_star, ast);
    // Promise.resolve()
    let promise_resolve = call_at(member_at("Promise", "resolve", span, ast), None);
    // Promise.resolve().then(() => …)
    let then: Expression<'a> = ast
        .member_expression_static(
            span,
            promise_resolve,
            ast.identifier_name(span, "then"),
            false,
        )
        .into();
    call_at(then, Some(factory))
}

/// `(0, expr)` sequence — strips `this` from a method-style callee.
fn sequence_zero<'a>(expr: Expression<'a>, ast: AstBuilder<'a>) -> Expression<'a> {
    let mut items = ast.vec();
    items.push(ast.expression_numeric_literal(
        SPAN,
        0.0,
        None,
        oxc_syntax::number::NumberBase::Decimal,
    ));
    items.push(expr);
    ast.expression_sequence(SPAN, items)
}

/// Builds `const <name> = …;`, stamping the statement with `span` (used for
/// `const m_1 = require("…")`, so the declaration maps to the original import).
fn const_decl_at<'a>(
    name: &str,
    init: Expression<'a>,
    span: oxc_span::Span,
    ast: AstBuilder<'a>,
) -> Statement<'a> {
    let id = ast.binding_pattern_binding_identifier(span, ast.allocator.alloc_str(name));
    let declarator = ast.variable_declarator(
        span,
        VariableDeclarationKind::Const,
        id,
        NONE,
        Some(init),
        false,
    );
    let mut decls = ast.vec();
    decls.push(declarator);
    Statement::from(ast.declaration_variable(span, VariableDeclarationKind::Const, decls, false))
}

/// `exports.<name> = <value>;`
fn assign_export_stmt<'a>(name: &str, value: Expression<'a>, ast: AstBuilder<'a>) -> Statement<'a> {
    let target = ast.member_expression_static(
        SPAN,
        ident("exports", ast),
        ast.identifier_name(SPAN, ast.allocator.alloc_str(name)),
        false,
    );
    let assign = ast.expression_assignment(
        SPAN,
        oxc_ast::ast::AssignmentOperator::Assign,
        AssignmentTarget::from(target),
        value,
    );
    ast.statement_expression(SPAN, assign)
}

/// Wrap a write to a live exported binding: `<inner>` → `exports.<name> = (<inner>)`.
/// `<inner>` is the original (already-traversed) assignment expression, so its LHS
/// still updates the module-local `let`/`var` (the read source-of-truth) while the
/// result is mirrored into `exports.<name>`.
fn export_write_wrap<'a>(name: &str, inner: Expression<'a>, ast: AstBuilder<'a>) -> Expression<'a> {
    let target = ast.member_expression_static(
        SPAN,
        ident("exports", ast),
        ast.identifier_name(SPAN, ast.allocator.alloc_str(name)),
        false,
    );
    ast.expression_assignment(
        SPAN,
        AssignmentOperator::Assign,
        AssignmentTarget::from(target),
        inner,
    )
}

/// Wrap an update to a live exported binding, preserving prefix/postfix value
/// semantics. The update has already been normalized to *prefix* (`++x`/`--x`):
///   - source prefix  → `exports.x = ++x` (value is the new value, correct)
///   - source postfix → `(exports.x = ++x) - 1` for `++` (`+ 1` for `--`), so the
///     overall expression still yields the OLD value while `exports.x` and the
///     local both hold the new value.
fn export_update_wrap<'a>(
    name: &str,
    inner: Expression<'a>,
    was_prefix: bool,
    op: UpdateOperator,
    ast: AstBuilder<'a>,
) -> Expression<'a> {
    let wrapped = export_write_wrap(name, inner, ast);
    if was_prefix {
        return wrapped;
    }
    // Postfix: compensate by the opposite of the applied delta.
    let (compensate_op, _) = match op {
        UpdateOperator::Increment => (oxc_ast::ast::BinaryOperator::Subtraction, 1.0),
        UpdateOperator::Decrement => (oxc_ast::ast::BinaryOperator::Addition, 1.0),
    };
    let one =
        ast.expression_numeric_literal(SPAN, 1.0, None, oxc_syntax::number::NumberBase::Decimal);
    ast.expression_binary(SPAN, wrapped, compensate_op, one)
}

/// `() => <value>` arrow (used as a re-export getter body).
fn arrow_getter<'a>(value: Expression<'a>, ast: AstBuilder<'a>) -> Expression<'a> {
    let params = ast.formal_parameters(
        SPAN,
        FormalParameterKind::ArrowFormalParameters,
        ast.vec(),
        NONE,
    );
    let mut stmts = ast.vec();
    stmts.push(ast.statement_expression(SPAN, value));
    let body = ast.function_body(SPAN, ast.vec(), stmts);
    ast.expression_arrow_function(SPAN, true, false, NONE, params, NONE, body)
}

/// `Object.defineProperty(exports, "<name>", { enumerable: true, configurable: true, get: () => <value> });`
///
/// Used for re-exports so they resolve lazily — required for circular module
/// graphs (e.g. ngrx selector barrels), matching TypeScript's `export … from`.
/// The descriptor is `configurable: true` (a deliberate deviation from tsc, which
/// emits a non-configurable getter): an `import * as ns` of a barrel copies this
/// descriptor onto the namespace member via `__importStar`/`__createBinding`, and
/// `jest.spyOn(ns, name)` then needs to `Object.defineProperty` over it — which
/// requires the member be configurable. Mirrors the R12 fix for `__createBinding`.
fn define_export_getter<'a>(
    name: &str,
    value: Expression<'a>,
    ast: AstBuilder<'a>,
) -> Statement<'a> {
    let mut props = ast.vec();
    props.push(ast.object_property_kind_object_property(
        SPAN,
        PropertyKind::Init,
        PropertyKey::StaticIdentifier(ast.alloc_identifier_name(SPAN, "enumerable")),
        ast.expression_boolean_literal(SPAN, true),
        false,
        false,
        false,
    ));
    props.push(ast.object_property_kind_object_property(
        SPAN,
        PropertyKind::Init,
        PropertyKey::StaticIdentifier(ast.alloc_identifier_name(SPAN, "configurable")),
        ast.expression_boolean_literal(SPAN, true),
        false,
        false,
        false,
    ));
    props.push(ast.object_property_kind_object_property(
        SPAN,
        PropertyKind::Init,
        PropertyKey::StaticIdentifier(ast.alloc_identifier_name(SPAN, "get")),
        arrow_getter(value, ast),
        false,
        false,
        false,
    ));
    let opts = ast.expression_object(SPAN, props);
    let name_lit = ast.expression_string_literal(SPAN, ast.allocator.alloc_str(name), None);
    let define = member("Object", "defineProperty", ast);
    let call_expr = call(define, vec![ident("exports", ast), name_lit, opts], ast);
    ast.statement_expression(SPAN, call_expr)
}

/// `Object.defineProperty(exports, "__esModule", { value: true });`
fn es_module_marker<'a>(ast: AstBuilder<'a>) -> Statement<'a> {
    let define = member("Object", "defineProperty", ast);
    let props = {
        let mut v = ast.vec();
        v.push(ast.object_property_kind_object_property(
            SPAN,
            oxc_ast::ast::PropertyKind::Init,
            oxc_ast::ast::PropertyKey::StaticIdentifier(ast.alloc_identifier_name(SPAN, "value")),
            ast.expression_boolean_literal(SPAN, true),
            false,
            false,
            false,
        ));
        ast.expression_object(SPAN, v)
    };
    let marker = ast.expression_string_literal(SPAN, "__esModule", None);
    let call_expr = call(define, vec![ident("exports", ast), marker, props], ast);
    ast.statement_expression(SPAN, call_expr)
}

/// `exports.a = exports.b = void 0;` (chained), matching TS's hoist.
fn void0_hoist<'a>(names: &[String], ast: AstBuilder<'a>) -> Statement<'a> {
    // Build right-to-left: void 0, then wrap each `exports.name = …`.
    let mut value = ast.expression_unary(
        SPAN,
        oxc_ast::ast::UnaryOperator::Void,
        ast.expression_numeric_literal(SPAN, 0.0, None, oxc_syntax::number::NumberBase::Decimal),
    );
    for name in names {
        let target = ast.member_expression_static(
            SPAN,
            ident("exports", ast),
            ast.identifier_name(SPAN, ast.allocator.alloc_str(name)),
            false,
        );
        value = ast.expression_assignment(
            SPAN,
            oxc_ast::ast::AssignmentOperator::Assign,
            AssignmentTarget::from(target),
            value,
        );
    }
    ast.statement_expression(SPAN, value)
}
