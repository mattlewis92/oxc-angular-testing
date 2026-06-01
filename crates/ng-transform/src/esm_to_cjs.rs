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
    Argument, AssignmentTarget, BindingPattern, Expression, FormalParameterKind,
    ImportDeclarationSpecifier, Program, PropertyKey, PropertyKind, Statement,
    VariableDeclarationKind,
};
use oxc_semantic::{Scoping, SemanticBuilder};
use oxc_span::SPAN;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{Traverse, TraverseCtx, traverse_mut};

const IMPORT_DEFAULT: &str = "var __importDefault = (this && this.__importDefault) || function (mod) {\n    return (mod && mod.__esModule) ? mod : { \"default\": mod };\n};\n";

const IMPORT_STAR: &str = r#"var __createBinding = (this && this.__createBinding) || (Object.create ? (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    var desc = Object.getOwnPropertyDescriptor(m, k);
    if (!desc || ("get" in desc ? !m.__esModule : desc.writable || desc.configurable)) {
      desc = { enumerable: true, get: function() { return m[k]; } };
    }
    Object.defineProperty(o, k2, desc);
}) : (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    o[k2] = m[k];
}));
var __setModuleDefault = (this && this.__setModuleDefault) || (Object.create ? (function(o, v) {
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

const EXPORT_STAR: &str = r#"var __createBinding = (this && this.__createBinding) || (Object.create ? (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    var desc = Object.getOwnPropertyDescriptor(m, k);
    if (!desc || ("get" in desc ? !m.__esModule : desc.writable || desc.configurable)) {
      desc = { enumerable: true, get: function() { return m[k]; } };
    }
    Object.defineProperty(o, k2, desc);
}) : (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    o[k2] = m[k];
}));
var __exportStar = (this && this.__exportStar) || function(m, exports) {
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
pub fn esm_to_cjs<'a>(allocator: &'a Allocator, program: &mut Program<'a>) -> String {
    let ast = AstBuilder::new(allocator);

    // 1. Assign a module var per source and a replacement per default/named local.
    let mut module_vars: HashMap<String, String> = HashMap::new();
    let mut used_names: HashMap<String, u32> = HashMap::new();
    let mut replacements: HashMap<String, Replacement> = HashMap::new();
    let mut needs = HelperNeeds::default();

    for stmt in &program.body {
        let Statement::ImportDeclaration(import) = stmt else {
            continue;
        };
        let source = import.source.value.as_str().to_string();
        let Some(specifiers) = &import.specifiers else {
            continue; // side-effect only
        };
        let has_namespace = specifiers
            .iter()
            .any(|s| matches!(s, ImportDeclarationSpecifier::ImportNamespaceSpecifier(_)));
        // A namespace import names the var after the local namespace binding.
        if let Some(ImportDeclarationSpecifier::ImportNamespaceSpecifier(ns)) = specifiers
            .iter()
            .find(|s| matches!(s, ImportDeclarationSpecifier::ImportNamespaceSpecifier(_)))
        {
            module_vars
                .entry(source.clone())
                .or_insert_with(|| ns.local.name.as_str().to_string());
        }
        let ns_var = module_vars
            .entry(source.clone())
            .or_insert_with(|| unique_module_var(&source, &mut used_names))
            .clone();
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
        let _ = has_namespace;
    }

    // 2. Rewrite references to default/named import locals (symbol-aware).
    if !replacements.is_empty() {
        let scoping = SemanticBuilder::new()
            .build(program)
            .semantic
            .into_scoping();
        let symbol_map = build_symbol_map(program, &scoping, &replacements);
        let mut rewriter = RefRewriter {
            symbol_map,
            replacements: replacements.clone(),
        };
        traverse_mut(&mut rewriter, allocator, program, scoping, ());
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
                rewrite_export_default(export.unbox(), &mut exported_names, ast, &mut body);
            }
            Statement::ExportAllDeclaration(export) => {
                needs.export_star = true;
                // `export * from "m"` → `__exportStar(require("m"), exports);`
                // (named `export * as ns` handled as a namespace export.)
                if let Some(name) = &export.exported {
                    // export * as ns from "m"
                    let req = require_call(export.source.value.as_str(), ast);
                    let star = call(ident("__importStar", ast), vec![req], ast);
                    needs.import_star = true;
                    body.push(assign_export_stmt(name.name().as_str(), star, ast));
                    exported_names.push(name.name().as_str().to_string());
                } else {
                    let req = require_call(export.source.value.as_str(), ast);
                    let call_expr = call(
                        ident("__exportStar", ast),
                        vec![req, ident("exports", ast)],
                        ast,
                    );
                    body.push(ast.statement_expression(SPAN, call_expr));
                }
            }
            other => body.push(other),
        }
    }

    // 4. Header in the AST: `__esModule` marker + `exports.x = void 0;` hoist.
    //    Interop helpers are returned as a text prelude (see below).
    let mut final_body = ast.vec();
    final_body.push(es_module_marker(ast));
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
    if needs.import_star {
        prelude.push_str(IMPORT_STAR);
    }
    if needs.export_star {
        prelude.push_str(EXPORT_STAR);
    }
    prelude
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
    _scoping: &Scoping,
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

struct RefRewriter {
    symbol_map: HashMap<SymbolId, Replacement>,
    replacements: HashMap<String, Replacement>,
}

impl<'a> Traverse<'a, ()> for RefRewriter {
    fn enter_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut TraverseCtx<'a, ()>) {
        // Wrap import-binding calls as `(0, ns.member)(...)` to drop `this`.
        if let Expression::CallExpression(call_expr) = expr
            && let Some(repl) = self.callee_replacement(&call_expr.callee, ctx)
        {
            let ast = ctx.ast;
            let member = member(&repl.ns_var, &repl.member, ast);
            call_expr.callee = sequence_zero(member, ast);
            return;
        }
        let Expression::Identifier(id) = expr else {
            return;
        };
        let Some(repl) = self.reference_replacement(id.reference_id.get(), ctx) else {
            return;
        };
        *expr = member(&repl.ns_var, &repl.member, ctx.ast);
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

    fn callee_replacement(
        &self,
        callee: &Expression<'_>,
        ctx: &TraverseCtx<'_, ()>,
    ) -> Option<Replacement> {
        let Expression::Identifier(id) = callee else {
            return None;
        };
        // Resolve by symbol; fall back to name when unresolved (defensive).
        self.reference_replacement(id.reference_id.get(), ctx)
            .or_else(|| self.replacements.get(id.name.as_str()).cloned())
    }
}

// --- statement rewriters ---------------------------------------------------

fn rewrite_import<'a>(
    import: &oxc_ast::ast::ImportDeclaration<'a>,
    module_vars: &HashMap<String, String>,
    needs: &mut HelperNeeds,
    emitted: &mut std::collections::HashSet<String>,
    ast: AstBuilder<'a>,
    out: &mut Vec<Statement<'a>>,
) {
    let source = import.source.value.as_str();
    let Some(specifiers) = &import.specifiers else {
        // side-effect import → `require("…");` (skip if already required).
        if emitted.insert(source.to_string()) {
            out.push(ast.statement_expression(SPAN, require_call(source, ast)));
        }
        return;
    };
    let ns_var = module_vars
        .get(source)
        .cloned()
        .unwrap_or_else(|| source.to_string());
    // A default import via `import x from` OR `import { default as x }`.
    let has_default = specifiers.iter().any(|s| match s {
        ImportDeclarationSpecifier::ImportDefaultSpecifier(_) => true,
        ImportDeclarationSpecifier::ImportSpecifier(spec) => {
            spec.imported.name().as_str() == "default"
        }
        ImportDeclarationSpecifier::ImportNamespaceSpecifier(_) => false,
    });
    // A non-default named import (`import { foo }`).
    let has_named = specifiers.iter().any(|s| match s {
        ImportDeclarationSpecifier::ImportSpecifier(spec) => {
            spec.imported.name().as_str() != "default"
        }
        _ => false,
    });
    let has_namespace = specifiers
        .iter()
        .any(|s| matches!(s, ImportDeclarationSpecifier::ImportNamespaceSpecifier(_)));

    if !emitted.insert(source.to_string()) {
        // Already required (e.g. also re-exported) — references use the same var.
        return;
    }
    let req = require_call(source, ast);
    // Match tsc's esModuleInterop: namespace or mixed default+named → __importStar
    // (the namespace is needed for both); default only → __importDefault; named
    // only → plain require.
    let init = if has_namespace || (has_default && has_named) {
        needs.import_star = true;
        call(ident("__importStar", ast), vec![req], ast)
    } else if has_default {
        needs.import_default = true;
        call(ident("__importDefault", ast), vec![req], ast)
    } else {
        req
    };
    out.push(const_decl(&ns_var, init, ast));
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
            out.push(const_decl(&ns_var, require_call(src, ast), ast));
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
    ast: AstBuilder<'a>,
    out: &mut Vec<Statement<'a>>,
) {
    use oxc_ast::ast::ExportDefaultDeclarationKind as K;
    exported_names.push("default".to_string());
    match export.declaration {
        K::FunctionDeclaration(func) => {
            let name = func.id.as_ref().map(|i| i.name.as_str().to_string());
            out.push(Statement::FunctionDeclaration(func));
            let value = name.map_or_else(|| undefined(ast), |n| ident(&n, ast));
            out.push(assign_export_stmt("default", value, ast));
        }
        K::ClassDeclaration(class) => {
            let name = class.id.as_ref().map(|i| i.name.as_str().to_string());
            out.push(Statement::ClassDeclaration(class));
            let value = name.map_or_else(|| undefined(ast), |n| ident(&n, ast));
            out.push(assign_export_stmt("default", value, ast));
        }
        expr => {
            // An expression: `export default <expr>;`
            let expression = expr.into_expression();
            out.push(assign_export_stmt("default", expression, ast));
        }
    }
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

fn undefined<'a>(ast: AstBuilder<'a>) -> Expression<'a> {
    ident("undefined", ast)
}

fn member<'a>(object: &str, property: &str, ast: AstBuilder<'a>) -> Expression<'a> {
    ast.member_expression_static(
        SPAN,
        ident(object, ast),
        ast.identifier_name(SPAN, ast.allocator.alloc_str(property)),
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

fn require_call<'a>(source: &str, ast: AstBuilder<'a>) -> Expression<'a> {
    let arg = ast.expression_string_literal(SPAN, ast.allocator.alloc_str(source), None);
    call(ident("require", ast), vec![arg], ast)
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

fn const_decl<'a>(name: &str, init: Expression<'a>, ast: AstBuilder<'a>) -> Statement<'a> {
    let id = ast.binding_pattern_binding_identifier(SPAN, ast.allocator.alloc_str(name));
    let declarator = ast.variable_declarator(
        SPAN,
        VariableDeclarationKind::Const,
        id,
        NONE,
        Some(init),
        false,
    );
    let mut decls = ast.vec();
    decls.push(declarator);
    Statement::from(ast.declaration_variable(SPAN, VariableDeclarationKind::Const, decls, false))
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

/// `Object.defineProperty(exports, "<name>", { enumerable: true, get: () => <value> });`
///
/// Used for re-exports so they resolve lazily — required for circular module
/// graphs (e.g. ngrx selector barrels), matching TypeScript's `export … from`.
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
