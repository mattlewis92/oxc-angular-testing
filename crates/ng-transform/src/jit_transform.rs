//! Angular compiler-cli JIT transforms, ported from
//! `@angular/compiler-cli/src/ngtsc/transform/jit` (`angularJitApplicationTransform`).
//!
//! Two transforms run, in order, in a single class traversal:
//!
//! 1. **Initializer-API transform** (`getInitializerApiJitTransform`): for
//!    `@Component`/`@Directive` classes, members initialized with the signal
//!    APIs gain a synthesized Angular decorator:
//!    - `input()` / `input.required()` → `@Input({ isSignal: true, alias, required, transform: undefined })`
//!    - `output()` → `@Output("<alias>")`
//!    - `model()` / `model.required()` → `@Input({ isSignal: true, alias, required })` + `@Output("<alias>Change")`
//!    - `viewChild[ren]()` / `contentChild[ren]()` → `@ViewChild`/etc.`(<locator>, { isSignal: true })`
//!
//! 2. **Downlevel-decorators transform** (`getDownlevelDecoratorsTransform`):
//!    - Class-level Angular decorators are **left as real decorators** (lowered
//!      later by `oxc_transformer` via `__decorate`).
//!    - Angular decorators on **constructor parameters** are moved to a
//!      synthesized `static ctorParameters = () => [{ type, decorators? }, ...]`
//!      and removed from the params.
//!    - Angular decorators on **members** are moved to a synthesized
//!      `static propDecorators = { name: [{ type, args? }, ...] }` and removed
//!      from the members.
//!    - Non-Angular decorators are left in place.
//!
//! A decorator/symbol is "Angular" when it resolves to an `@angular/core`
//! import (tracked syntactically: named specifiers incl. aliases, and the
//! `import * as ng` namespace).

use std::collections::{HashMap, HashSet};

use oxc_allocator::CloneIn;
use oxc_ast::AstBuilder;
use oxc_ast::NONE;
use oxc_ast::ast::{
    Argument, ArrayExpressionElement, CallExpression, Class, ClassElement, Declaration, Decorator,
    Expression, FormalParameter, FormalParameterKind, ImportDeclarationSpecifier,
    ImportOrExportKind, MethodDefinitionKind, ObjectPropertyKind, Program, PropertyDefinitionType,
    PropertyKey, PropertyKind, Statement, TSType, TSTypeName, WithClause,
};
use oxc_span::SPAN;
use oxc_traverse::{Traverse, TraverseCtx};

const ANGULAR_CORE: &str = "@angular/core";
const NG_CLASS_DECORATORS: &[&str] = &["Component", "Directive", "Injectable", "Pipe", "NgModule"];

/// The signal initializer API a class field is initialized with.
#[derive(Clone, Copy)]
enum Initializer {
    Input { required: bool },
    Model { required: bool },
    Output,
    Query(&'static str), // ViewChild / ViewChildren / ContentChild / ContentChildren
}

pub struct JitTransform {
    /// local identifier → imported `@angular/core` name (named specifiers).
    ng_locals: HashMap<String, String>,
    /// local name of `import * as <ns> from '@angular/core'`, if any.
    ng_namespace: Option<String>,
    /// canonical `@angular/core` names we synthesized and must ensure are imported.
    needed_imports: HashSet<String>,
    /// Names bound to a **runtime value** (class / enum / function / var / value
    /// import). A constructor-parameter type is only emitted as a value
    /// reference in `ctorParameters` when its name is one of these — otherwise
    /// (interfaces, type aliases, `import type`, utility types like `Pick` /
    /// `ReadonlyArray`) it would be a `ReferenceError` at DI time, so we emit
    /// `Object`, matching tsc's `emitDecoratorMetadata`.
    value_names: HashSet<String>,
    pub changed: bool,
}

impl JitTransform {
    #[must_use]
    pub fn new() -> Self {
        Self {
            ng_locals: HashMap::new(),
            ng_namespace: None,
            needed_imports: HashSet::new(),
            value_names: HashSet::new(),
            changed: false,
        }
    }

    /// The `@angular/core` imported name a decorator/call identifier resolves to,
    /// if any. Handles bare identifiers (incl. aliases) and `ng.Foo` namespace
    /// access.
    fn ng_name<'a>(&self, expr: &Expression<'a>) -> Option<String> {
        match expr {
            Expression::Identifier(id) => self.ng_locals.get(id.name.as_str()).cloned(),
            Expression::StaticMemberExpression(m) => {
                if let Expression::Identifier(obj) = &m.object
                    && self.ng_namespace.as_deref() == Some(obj.name.as_str())
                {
                    Some(m.property.name.as_str().to_string())
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn decorator_ng_name<'a>(&self, dec: &Decorator<'a>) -> Option<String> {
        match &dec.expression {
            Expression::CallExpression(call) => self.ng_name(&call.callee),
            other => self.ng_name(other),
        }
    }

    fn class_is_angular<'a>(&self, class: &Class<'a>) -> bool {
        class.decorators.iter().any(|d| {
            self.decorator_ng_name(d)
                .is_some_and(|n| NG_CLASS_DECORATORS.contains(&n.as_str()))
        })
    }

    /// Register a canonical `@angular/core` symbol so it's treated as Angular and
    /// imported in `exit_program`.
    fn use_ng_symbol(&mut self, name: &str) {
        self.ng_locals.insert(name.to_string(), name.to_string());
        self.needed_imports.insert(name.to_string());
    }
}

impl Default for JitTransform {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Small AST builders
// ---------------------------------------------------------------------------

fn key_name<'a>(key: &PropertyKey<'a>) -> Option<String> {
    match key {
        PropertyKey::StaticIdentifier(id) => Some(id.name.as_str().to_string()),
        PropertyKey::StringLiteral(s) => Some(s.value.as_str().to_string()),
        _ => None,
    }
}

/// The underlying initializer `CallExpression` of a class-field value, seeing
/// through a coverage-inserted counter wrapper. Source-level coverage rewrites
/// `foo = input(...)` to `foo = (++cov.s[N], input(...))` (a `SequenceExpression`
/// whose last element is the real initializer) and may parenthesize it; the
/// signal-API detector must look through both, else instrumented component files
/// lose their synthesized signal `propDecorators`.
fn initializer_call<'a, 'b>(value: &'b Expression<'a>) -> Option<&'b CallExpression<'a>> {
    match value {
        Expression::CallExpression(call) => Some(call),
        Expression::SequenceExpression(seq) => seq.expressions.last().and_then(initializer_call),
        Expression::ParenthesizedExpression(p) => initializer_call(&p.expression),
        _ => None,
    }
}

fn ident<'a>(name: &str, ast: AstBuilder<'a>) -> Expression<'a> {
    ast.expression_identifier(SPAN, ast.allocator.alloc_str(name))
}

fn undefined<'a>(ast: AstBuilder<'a>) -> Expression<'a> {
    ident("undefined", ast)
}

fn string_lit<'a>(value: &str, ast: AstBuilder<'a>) -> Expression<'a> {
    ast.expression_string_literal(SPAN, ast.allocator.alloc_str(value), None)
}

fn bool_lit<'a>(value: bool, ast: AstBuilder<'a>) -> Expression<'a> {
    ast.expression_boolean_literal(SPAN, value)
}

fn prop<'a>(name: &str, value: Expression<'a>, ast: AstBuilder<'a>) -> ObjectPropertyKind<'a> {
    ast.object_property_kind_object_property(
        SPAN,
        PropertyKind::Init,
        PropertyKey::StaticIdentifier(
            ast.alloc_identifier_name(SPAN, ast.allocator.alloc_str(name)),
        ),
        value,
        false,
        false,
        false,
    )
}

fn object<'a>(props: Vec<ObjectPropertyKind<'a>>, ast: AstBuilder<'a>) -> Expression<'a> {
    let mut v = ast.vec();
    for p in props {
        v.push(p);
    }
    ast.expression_object(SPAN, v)
}

fn array<'a>(elements: Vec<Expression<'a>>, ast: AstBuilder<'a>) -> Expression<'a> {
    let mut v = ast.vec();
    for e in elements {
        v.push(ArrayExpressionElement::from(e));
    }
    ast.expression_array(SPAN, v)
}

/// `() => <body>` (expression-bodied arrow).
fn arrow<'a>(body: Expression<'a>, ast: AstBuilder<'a>) -> Expression<'a> {
    let params = ast.formal_parameters(
        SPAN,
        FormalParameterKind::ArrowFormalParameters,
        ast.vec(),
        NONE,
    );
    let mut stmts = ast.vec();
    stmts.push(ast.statement_expression(SPAN, body));
    let fn_body = ast.function_body(SPAN, ast.vec(), stmts);
    ast.expression_arrow_function(SPAN, true, false, NONE, params, NONE, fn_body)
}

/// Build a `static <name> = <value>;` class element.
fn static_property<'a>(name: &str, value: Expression<'a>, ast: AstBuilder<'a>) -> ClassElement<'a> {
    let key = PropertyKey::StaticIdentifier(
        ast.alloc_identifier_name(SPAN, ast.allocator.alloc_str(name)),
    );
    ast.class_element_property_definition(
        SPAN,
        PropertyDefinitionType::PropertyDefinition,
        ast.vec(),
        key,
        NONE,
        Some(value),
        false, // computed
        true,  // static
        false, // declare
        false, // override
        false, // optional
        false, // definite
        false, // readonly
        None,  // accessibility
    )
}

// ---------------------------------------------------------------------------
// Type → runtime expression (for `ctorParameters` `type`)
// ---------------------------------------------------------------------------

fn type_name_to_expr<'a>(name: &TSTypeName<'a>, ast: AstBuilder<'a>) -> Option<Expression<'a>> {
    match name {
        TSTypeName::IdentifierReference(id) => Some(ident(id.name.as_str(), ast)),
        TSTypeName::QualifiedName(q) => {
            let object = type_name_to_expr(&q.left, ast)?;
            Some(
                ast.member_expression_static(
                    SPAN,
                    object,
                    ast.identifier_name(SPAN, ast.allocator.alloc_str(q.right.name.as_str())),
                    false,
                )
                .into(),
            )
        }
        TSTypeName::ThisExpression(_) => None,
    }
}

/// The leftmost identifier of a (possibly-qualified) type name, e.g. `ns` in
/// `ns.Service` or `Pick` in `Pick<…>`.
fn type_root_name<'a>(name: &'a TSTypeName<'a>) -> Option<&'a str> {
    match name {
        TSTypeName::IdentifierReference(id) => Some(id.name.as_str()),
        TSTypeName::QualifiedName(q) => type_root_name(&q.left),
        TSTypeName::ThisExpression(_) => None,
    }
}

fn type_to_expr<'a>(
    ts_type: &TSType<'a>,
    value_names: &HashSet<String>,
    ast: AstBuilder<'a>,
) -> Expression<'a> {
    match ts_type {
        TSType::TSTypeReference(r) => {
            // Emit the value reference only when the type's root name resolves to
            // a runtime value (a class/enum still in scope after type elision).
            // Utility types (`Pick`/`Omit`/…), structural types (`ReadonlyArray`/
            // `Array<T>`), interfaces, type aliases and `import type` symbols have
            // no runtime value, so emit `Object` (matching tsc's
            // `emitDecoratorMetadata`) rather than a dangling reference.
            match type_root_name(&r.type_name) {
                Some(name) if value_names.contains(name) => {
                    type_name_to_expr(&r.type_name, ast).unwrap_or_else(|| ident("Object", ast))
                }
                _ => ident("Object", ast),
            }
        }
        TSType::TSStringKeyword(_) => ident("String", ast),
        TSType::TSNumberKeyword(_) => ident("Number", ast),
        TSType::TSBooleanKeyword(_) => ident("Boolean", ast),
        TSType::TSUnionType(u) => {
            let non_null: Vec<&TSType<'a>> = u
                .types
                .iter()
                .filter(|t| !matches!(t, TSType::TSNullKeyword(_) | TSType::TSUndefinedKeyword(_)))
                .collect();
            if non_null.len() == 1 {
                type_to_expr(non_null[0], value_names, ast)
            } else {
                undefined(ast)
            }
        }
        _ => undefined(ast),
    }
}

/// Collect names bound to a runtime value (value imports, class/enum
/// declarations, bare or exported) for [`type_to_expr`].
/// `true` if `import` has a namespace specifier (`import * as ns from …`). Such a
/// declaration cannot also carry named specifiers, so synthesized symbols must go
/// in a separate `import { … }` statement rather than be merged into it.
fn import_has_namespace(import: &oxc_ast::ast::ImportDeclaration<'_>) -> bool {
    import.specifiers.as_ref().is_some_and(|specs| {
        specs
            .iter()
            .any(|s| matches!(s, ImportDeclarationSpecifier::ImportNamespaceSpecifier(_)))
    })
}

fn collect_value_names(stmt: &Statement<'_>, out: &mut HashSet<String>) {
    match stmt {
        Statement::ImportDeclaration(import) => {
            if matches!(import.import_kind, ImportOrExportKind::Type) {
                return; // `import type { … }` — type-only, elided.
            }
            let Some(specs) = &import.specifiers else {
                return;
            };
            for spec in specs {
                match spec {
                    ImportDeclarationSpecifier::ImportSpecifier(s)
                        if !matches!(s.import_kind, ImportOrExportKind::Type) =>
                    {
                        out.insert(s.local.name.as_str().to_string());
                    }
                    ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => {
                        out.insert(s.local.name.as_str().to_string());
                    }
                    ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => {
                        out.insert(s.local.name.as_str().to_string());
                    }
                    ImportDeclarationSpecifier::ImportSpecifier(_) => {} // `import { type X }`
                }
            }
        }
        Statement::ClassDeclaration(c) => {
            if let Some(id) = &c.id {
                out.insert(id.name.as_str().to_string());
            }
        }
        Statement::TSEnumDeclaration(e) => {
            out.insert(e.id.name.as_str().to_string());
        }
        Statement::ExportNamedDeclaration(e) => match &e.declaration {
            Some(Declaration::ClassDeclaration(c)) => {
                if let Some(id) = &c.id {
                    out.insert(id.name.as_str().to_string());
                }
            }
            Some(Declaration::TSEnumDeclaration(en)) => {
                out.insert(en.id.name.as_str().to_string());
            }
            _ => {}
        },
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Decorator → `{ type, args? }` metadata object
// ---------------------------------------------------------------------------

/// Consume a decorator and produce its `{ type: <fn>, args?: [...] }` metadata
/// object for `propDecorators` / `ctorParameters`.
fn decorator_metadata<'a>(dec: Decorator<'a>, ast: AstBuilder<'a>) -> Expression<'a> {
    match dec.expression {
        Expression::CallExpression(call) => {
            let call = call.unbox();
            let mut props = vec![prop("type", call.callee, ast)];
            if !call.arguments.is_empty() {
                let mut elems = ast.vec();
                for arg in call.arguments {
                    elems.push(ArrayExpressionElement::from(arg));
                }
                props.push(prop("args", ast.expression_array(SPAN, elems), ast));
            }
            object(props, ast)
        }
        other => object(vec![prop("type", other, ast)], ast),
    }
}

impl<'a> Traverse<'a, ()> for JitTransform {
    fn enter_program(&mut self, node: &mut Program<'a>, _ctx: &mut TraverseCtx<'a, ()>) {
        for stmt in &node.body {
            // Track runtime-value bindings (for ctorParameters type emission).
            collect_value_names(stmt, &mut self.value_names);

            let Statement::ImportDeclaration(import) = stmt else {
                continue;
            };
            if import.source.value.as_str() != ANGULAR_CORE {
                continue;
            }
            let Some(specifiers) = &import.specifiers else {
                continue;
            };
            for spec in specifiers {
                match spec {
                    ImportDeclarationSpecifier::ImportSpecifier(s) => {
                        self.ng_locals.insert(
                            s.local.name.as_str().to_string(),
                            s.imported.name().as_str().to_string(),
                        );
                    }
                    ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => {
                        self.ng_namespace = Some(s.local.name.as_str().to_string());
                    }
                    ImportDeclarationSpecifier::ImportDefaultSpecifier(_) => {}
                }
            }
        }
    }

    // Runs on `exit` (after the class body has been walked) so the synthesized
    // `static ctorParameters = () => [...]` / `propDecorators` nodes — which the
    // traverser has no scope registered for — are never walked. The post-JIT
    // `SemanticBuilder` rebuild assigns their scopes for the lowering pass.
    fn exit_class(&mut self, node: &mut Class<'a>, ctx: &mut TraverseCtx<'a, ()>) {
        if self.ng_locals.is_empty() && self.ng_namespace.is_none() {
            return;
        }
        let ast = ctx.ast;
        let class_is_angular = self.class_is_angular(node);

        // --- 1. Initializer-API transform: synthesize decorators for signals.
        if class_is_angular {
            self.synthesize_signal_decorators(node, ast);
        }

        // --- 2a. Downlevel constructor parameter decorators → ctorParameters.
        let ctor_params = self.downlevel_constructor(node, ast);
        let capture_ctor = class_is_angular
            || ctor_params
                .as_ref()
                .is_some_and(|p| p.iter().any(|el| el.1));
        let ctor_property = match (capture_ctor, ctor_params) {
            (true, Some(params)) => {
                let elements: Vec<Expression<'a>> = params.into_iter().map(|(el, _)| el).collect();
                Some(static_property(
                    "ctorParameters",
                    arrow(array(elements, ast), ast),
                    ast,
                ))
            }
            _ => None,
        };

        // --- 2b. Downlevel member decorators → propDecorators.
        let prop_decorators = self.downlevel_members(node, ast);

        if let Some(ctor) = ctor_property {
            self.changed = true;
            node.body.body.push(ctor);
        }
        if let Some(props) = prop_decorators {
            self.changed = true;
            node.body
                .body
                .push(static_property("propDecorators", object(props, ast), ast));
        }
    }

    fn exit_program(&mut self, node: &mut Program<'a>, ctx: &mut TraverseCtx<'a, ()>) {
        if self.needed_imports.is_empty() {
            return;
        }
        let ast = ctx.ast;
        let mut to_add: Vec<String> = self.needed_imports.iter().cloned().collect();
        // Keep only names not present as an existing imported specifier.
        let already: HashSet<String> = node
            .body
            .iter()
            .filter_map(|s| match s {
                Statement::ImportDeclaration(i) if i.source.value.as_str() == ANGULAR_CORE => {
                    i.specifiers.as_ref()
                }
                _ => None,
            })
            .flatten()
            .filter_map(|spec| match spec {
                ImportDeclarationSpecifier::ImportSpecifier(s) => {
                    Some(s.imported.name().as_str().to_string())
                }
                _ => None,
            })
            .collect();
        to_add.retain(|n| !already.contains(n));
        to_add.sort();
        if to_add.is_empty() {
            return;
        }

        // Merge the synthesized named specifiers into the first NAMED @angular/core
        // import (a default + named import is valid). A namespace import
        // (`import * as ng from '@angular/core'`) cannot carry named specifiers in
        // the same declaration — `import * as ng, { Input }` is a SyntaxError — so
        // skip it and emit a dedicated `import { … } from '@angular/core'` below.
        let existing = node.body.iter_mut().find_map(|s| match s {
            Statement::ImportDeclaration(i)
                if i.source.value.as_str() == ANGULAR_CORE && !import_has_namespace(i) =>
            {
                Some(i)
            }
            _ => None,
        });
        match existing {
            Some(import) => {
                let specifiers = import.specifiers.get_or_insert_with(|| ast.vec());
                for name in &to_add {
                    let arena = ast.allocator.alloc_str(name);
                    specifiers.push(ast.import_declaration_specifier_import_specifier(
                        SPAN,
                        ast.module_export_name_identifier_name(SPAN, arena),
                        ast.binding_identifier(SPAN, arena),
                        ImportOrExportKind::Value,
                    ));
                }
            }
            None => {
                let mut specifiers = ast.vec();
                for name in &to_add {
                    let arena = ast.allocator.alloc_str(name);
                    specifiers.push(ast.import_declaration_specifier_import_specifier(
                        SPAN,
                        ast.module_export_name_identifier_name(SPAN, arena),
                        ast.binding_identifier(SPAN, arena),
                        ImportOrExportKind::Value,
                    ));
                }
                let source = ast.string_literal(SPAN, ANGULAR_CORE, None);
                let decl = ast.alloc_import_declaration(
                    SPAN,
                    Some(specifiers),
                    source,
                    None,
                    None::<WithClause>,
                    ImportOrExportKind::Value,
                );
                node.body.insert(0, Statement::ImportDeclaration(decl));
            }
        }
    }
}

impl JitTransform {
    /// For each signal-initialized member, synthesize and prepend the Angular
    /// decorator(s) onto the member.
    fn synthesize_signal_decorators<'a>(&mut self, node: &mut Class<'a>, ast: AstBuilder<'a>) {
        // Collect synthesized decorators per member index first (avoids borrow issues).
        let mut synthesized: Vec<(usize, Vec<Decorator<'a>>)> = Vec::new();
        for (idx, element) in node.body.body.iter().enumerate() {
            let ClassElement::PropertyDefinition(p) = element else {
                continue;
            };
            let Some(name) = key_name(&p.key) else {
                continue;
            };
            let Some(value) = &p.value else {
                continue;
            };
            // See through a coverage counter wrapper. Coverage instrumentation runs
            // first (on the source AST) and rewrites `foo = input(...)` to
            // `foo = (++cov.s[N], input(...))` — a sequence whose last element is the
            // real initializer. Signal-API detection must look through it, else an
            // instrumented component file (every file matched by `collectCoverageFrom`)
            // loses its synthesized signal-input/query `propDecorators` → `ɵcmp.inputs`
            // ends up empty (setInput no-ops, `input.required()` throws NG0950).
            let Some(call) = initializer_call(value) else {
                continue;
            };
            let Some(init) = self.classify_initializer(call) else {
                continue;
            };
            let alias = self.extract_alias(call).unwrap_or(name);
            let decs = self.build_signal_decorators(init, &alias, call, ast);
            if !decs.is_empty() {
                synthesized.push((idx, decs));
            }
        }
        for (idx, decs) in synthesized {
            if let ClassElement::PropertyDefinition(p) = &mut node.body.body[idx] {
                // Prepend so downlevel emits them first.
                let mut new = ast.vec();
                for d in decs {
                    new.push(d);
                }
                for d in std::mem::replace(&mut p.decorators, ast.vec()) {
                    new.push(d);
                }
                p.decorators = new;
            }
        }
    }

    /// Classify a `input()`/`output()`/`model()`/query call by its `@angular/core`
    /// callee. Returns `None` if it isn't an `@angular/core` initializer API.
    fn classify_initializer<'a>(
        &self,
        call: &oxc_ast::ast::CallExpression<'a>,
    ) -> Option<Initializer> {
        // `<base>.required(...)` → callee is `<base>.required`. Covers the signal
        // input/model required forms AND the required single-child queries
        // (`viewChild.required`/`contentChild.required`). The query metadata is
        // identical to the non-required variant — required-ness isn't encoded in
        // the decorator args (Angular reads it off the runtime RequiredSignal), so
        // the only thing that matters is that we don't skip the `.required` member
        // call when collecting query metadata (else the query gets no
        // `propDecorators` entry and Angular never registers it → NG0951).
        if let Expression::StaticMemberExpression(m) = &call.callee
            && m.property.name.as_str() == "required"
            && let Some(base) = self.ng_name(&m.object)
        {
            return match base.as_str() {
                "input" => Some(Initializer::Input { required: true }),
                "model" => Some(Initializer::Model { required: true }),
                "viewChild" => Some(Initializer::Query("ViewChild")),
                "contentChild" => Some(Initializer::Query("ContentChild")),
                _ => None,
            };
        }
        let name = self.ng_name(&call.callee)?;
        match name.as_str() {
            "input" => Some(Initializer::Input { required: false }),
            "model" => Some(Initializer::Model { required: false }),
            "output" => Some(Initializer::Output),
            "viewChild" => Some(Initializer::Query("ViewChild")),
            "viewChildren" => Some(Initializer::Query("ViewChildren")),
            "contentChild" => Some(Initializer::Query("ContentChild")),
            "contentChildren" => Some(Initializer::Query("ContentChildren")),
            _ => None,
        }
    }

    /// Find an `alias: '...'` string in any object-literal argument of the call.
    fn extract_alias<'a>(&self, call: &oxc_ast::ast::CallExpression<'a>) -> Option<String> {
        for arg in &call.arguments {
            if let Argument::ObjectExpression(obj) = arg {
                for p in &obj.properties {
                    if let ObjectPropertyKind::ObjectProperty(op) = p
                        && key_name(&op.key).as_deref() == Some("alias")
                        && let Expression::StringLiteral(s) = &op.value
                    {
                        return Some(s.value.as_str().to_string());
                    }
                }
            }
        }
        None
    }

    /// Extract the `transform: <fn>` value from any object-literal argument of an
    /// `input(...)` / `input.required(...)` call, cloned into the arena. Mirrors
    /// `extract_alias`; returns `None` when no `transform` option is present.
    fn extract_transform<'a>(
        &self,
        call: &CallExpression<'a>,
        ast: AstBuilder<'a>,
    ) -> Option<Expression<'a>> {
        for arg in &call.arguments {
            if let Argument::ObjectExpression(obj) = arg {
                for p in &obj.properties {
                    if let ObjectPropertyKind::ObjectProperty(op) = p
                        && key_name(&op.key).as_deref() == Some("transform")
                    {
                        return Some(op.value.clone_in(ast.allocator));
                    }
                }
            }
        }
        None
    }

    fn build_signal_decorators<'a>(
        &mut self,
        init: Initializer,
        alias: &str,
        call: &CallExpression<'a>,
        ast: AstBuilder<'a>,
    ) -> Vec<Decorator<'a>> {
        let mut decs = Vec::new();
        match init {
            Initializer::Input { required } => {
                let transform = self.extract_transform(call, ast);
                decs.push(self.input_decorator(alias, required, true, transform, ast));
            }
            Initializer::Model { required } => {
                // model() has no `transform` option, so emit no transform prop.
                decs.push(self.input_decorator(alias, required, false, None, ast));
                decs.push(self.output_decorator(&format!("{alias}Change"), ast));
            }
            Initializer::Output => {
                decs.push(self.output_decorator(alias, ast));
            }
            Initializer::Query(kind) => {
                decs.push(self.query_decorator(kind, call, ast));
            }
        }
        decs
    }

    fn input_decorator<'a>(
        &mut self,
        alias: &str,
        required: bool,
        with_transform: bool,
        transform: Option<Expression<'a>>,
        ast: AstBuilder<'a>,
    ) -> Decorator<'a> {
        self.use_ng_symbol("Input");
        let mut props = vec![
            prop("isSignal", bool_lit(true, ast), ast),
            prop("alias", string_lit(alias, ast), ast),
            prop("required", bool_lit(required, ast), ast),
        ];
        if with_transform {
            // Propagate the user's `transform` fn verbatim (mirrors Angular's
            // getInitializerApiJitTransform), falling back to `undefined` when none.
            let value = transform.unwrap_or_else(|| undefined(ast));
            props.push(prop("transform", value, ast));
        }
        let mut args = ast.vec();
        args.push(Argument::from(object(props, ast)));
        let call = ast.expression_call(SPAN, ident("Input", ast), NONE, args, false);
        ast.decorator(SPAN, call)
    }

    fn output_decorator<'a>(&mut self, binding: &str, ast: AstBuilder<'a>) -> Decorator<'a> {
        self.use_ng_symbol("Output");
        let mut args = ast.vec();
        args.push(Argument::from(string_lit(binding, ast)));
        let call = ast.expression_call(SPAN, ident("Output", ast), NONE, args, false);
        ast.decorator(SPAN, call)
    }

    /// `viewChild(locator, opts?)` → `@ViewChild(locator, { ...opts, isSignal: true })`,
    /// matching Angular's JIT downlevel: the predicate (first arg) is preserved
    /// verbatim, and `isSignal: true` is appended to a spread of the original
    /// options. Dropping the locator left Angular unable to wire the query
    /// (`this.query()` threw at runtime).
    fn query_decorator<'a>(
        &mut self,
        kind: &'static str,
        call: &CallExpression<'a>,
        ast: AstBuilder<'a>,
    ) -> Decorator<'a> {
        self.use_ng_symbol(kind);
        let alloc = ast.allocator;

        // Options: `{ ...<original options arg>, isSignal: true }`.
        let mut opts_props = ast.vec();
        if let Some(opts) = call.arguments.get(1).and_then(Argument::as_expression) {
            opts_props.push(ast.object_property_kind_spread_property(SPAN, opts.clone_in(alloc)));
        }
        opts_props.push(prop("isSignal", bool_lit(true, ast), ast));
        let opts_obj = ast.expression_object(SPAN, opts_props);

        let mut args = ast.vec();
        // Predicate: the first argument (class ref / string / forwardRef), as-is.
        if let Some(predicate) = call.arguments.first() {
            args.push(predicate.clone_in(alloc));
        }
        args.push(Argument::from(opts_obj));

        let decorator_call = ast.expression_call(SPAN, ident(kind, ast), NONE, args, false);
        ast.decorator(SPAN, decorator_call)
    }

    /// Process the constructor: strip Angular param decorators, collect
    /// `{ type, decorators? }` per param. Returns `Vec<(element, param_had_angular_decorator)>`.
    fn downlevel_constructor<'a>(
        &mut self,
        node: &mut Class<'a>,
        ast: AstBuilder<'a>,
    ) -> Option<Vec<(Expression<'a>, bool)>> {
        let ctor = node.body.body.iter_mut().find_map(|el| match el {
            ClassElement::MethodDefinition(m) if m.kind == MethodDefinitionKind::Constructor => {
                Some(m)
            }
            _ => None,
        })?;
        if ctor.value.params.items.is_empty() {
            return None;
        }
        let mut elements = Vec::new();
        for param in &mut ctor.value.params.items {
            elements.push(self.process_param(param, ast));
        }
        Some(elements)
    }

    /// Strip Angular decorators from a constructor param and build its
    /// `{ type, decorators? }` element. Returns `(element, had_angular_decorator)`.
    fn process_param<'a>(
        &mut self,
        param: &mut FormalParameter<'a>,
        ast: AstBuilder<'a>,
    ) -> (Expression<'a>, bool) {
        // Resolve the type before we touch decorators.
        let value_names = &self.value_names;
        let type_expr = param.type_annotation.as_ref().map_or_else(
            || undefined(ast),
            |ann| type_to_expr(&ann.type_annotation, value_names, ast),
        );

        let mut keep = ast.vec();
        let mut angular: Vec<Decorator<'a>> = Vec::new();
        for dec in std::mem::replace(&mut param.decorators, ast.vec()) {
            if self.decorator_ng_name(&dec).is_some() {
                angular.push(dec);
            } else {
                keep.push(dec);
            }
        }
        param.decorators = keep;

        let had_angular = !angular.is_empty();
        let mut props = vec![prop("type", type_expr, ast)];
        if had_angular {
            let metas: Vec<Expression<'a>> = angular
                .into_iter()
                .map(|d| decorator_metadata(d, ast))
                .collect();
            props.push(prop("decorators", array(metas, ast), ast));
        }
        (object(props, ast), had_angular)
    }

    /// Strip Angular decorators from members, collecting `propDecorators` entries.
    fn downlevel_members<'a>(
        &mut self,
        node: &mut Class<'a>,
        ast: AstBuilder<'a>,
    ) -> Option<Vec<ObjectPropertyKind<'a>>> {
        let mut entries: Vec<(String, Expression<'a>)> = Vec::new();
        for element in &mut node.body.body {
            // Read the member name first (immutable), then take a mutable borrow
            // of its decorators — two separate matches to avoid aliasing.
            let name = match &*element {
                ClassElement::PropertyDefinition(p) => key_name(&p.key),
                ClassElement::MethodDefinition(m)
                    if m.kind != MethodDefinitionKind::Constructor =>
                {
                    key_name(&m.key)
                }
                ClassElement::AccessorProperty(a) => key_name(&a.key),
                _ => continue,
            };
            let Some(name) = name else { continue };
            let decorators = match element {
                ClassElement::PropertyDefinition(p) => &mut p.decorators,
                ClassElement::MethodDefinition(m)
                    if m.kind != MethodDefinitionKind::Constructor =>
                {
                    &mut m.decorators
                }
                ClassElement::AccessorProperty(a) => &mut a.decorators,
                _ => continue,
            };
            if decorators.is_empty() {
                continue;
            }
            let mut keep = ast.vec();
            let mut metas: Vec<Expression<'a>> = Vec::new();
            for dec in std::mem::replace(decorators, ast.vec()) {
                if self.decorator_ng_name(&dec).is_some() {
                    metas.push(decorator_metadata(dec, ast));
                } else {
                    keep.push(dec);
                }
            }
            *decorators = keep;
            if !metas.is_empty() {
                entries.push((name, array(metas, ast)));
            }
        }
        if entries.is_empty() {
            return None;
        }
        Some(
            entries
                .into_iter()
                .map(|(name, value)| prop(&name, value, ast))
                .collect(),
        )
    }
}
