//! `replaceResources` pass — the jest-preset-angular component resource
//! transform, ported to an oxc [`Traverse`] pass.
//!
//! For classes decorated with `@Component({...})` (where `Component` resolves to
//! an `@angular/core` import):
//!
//! - `templateUrl: './x.html'` → `template: require('./x.html')` (require mode)
//!   or a hoisted `import __NG_CLI_RESOURCE__N from './x.html'` + `template:
//!   __NG_CLI_RESOURCE__N` (import mode).
//! - `styleUrls`, `styleUrl`, `styles`, `moduleId` → removed.
//!
//! Detection of the `@angular/core` origin is by import tracking (named
//! specifiers, including aliases). The TypeScript original uses the type
//! checker; this is the lightweight equivalent.

use std::collections::HashSet;

use oxc_ast::AstBuilder;
use oxc_ast::ast::{
    Argument, Class, Decorator, Expression, ImportDeclarationSpecifier, ImportOrExportKind,
    ObjectExpression, ObjectProperty, ObjectPropertyKind, Program, PropertyKey, Statement,
    TSTypeParameterInstantiation, WithClause,
};
use oxc_span::SPAN;
use oxc_traverse::{Traverse, TraverseCtx};

const RESOURCE_PREFIX: &str = "__NG_CLI_RESOURCE__";
const ANGULAR_CORE: &str = "@angular/core";

/// State for the resource transform.
pub struct ResourceTransform {
    /// Emit a hoisted top-level `import` instead of `require(...)`.
    use_import: bool,
    /// Local identifier names that refer to `Component` from `@angular/core`.
    component_locals: HashSet<String>,
    /// Local names of `import * as ng from '@angular/core'` namespace imports, so
    /// `@ng.Component({ templateUrl })` is recognized too (parity with jit_transform).
    component_namespaces: HashSet<String>,
    /// `import` mode: collected `(local_var, normalized_source)` to hoist.
    pending_imports: Vec<(String, String)>,
    /// Counter for `__NG_CLI_RESOURCE__N` names.
    counter: usize,
    /// Whether this pass changed anything (resources or imports).
    pub changed: bool,
}

impl ResourceTransform {
    #[must_use]
    pub fn new(use_import: bool) -> Self {
        Self {
            use_import,
            component_locals: HashSet::new(),
            component_namespaces: HashSet::new(),
            pending_imports: Vec::new(),
            counter: 0,
            changed: false,
        }
    }
}

/// Normalize a resource path the way jest-preset-angular does: relative paths
/// without a leading `.` get a `./` prefix; everything else is left alone.
fn normalize_url(url: &str) -> String {
    if url.starts_with('.') || url.starts_with('/') {
        url.to_string()
    } else {
        format!("./{url}")
    }
}

/// The name of an object-literal property key, if it is a plain identifier or
/// string key.
fn key_name<'a>(key: &PropertyKey<'a>) -> Option<&'a str> {
    match key {
        PropertyKey::StaticIdentifier(id) => Some(id.name.as_str()),
        PropertyKey::StringLiteral(s) => Some(s.value.as_str()),
        _ => None,
    }
}

impl<'a> ResourceTransform {
    /// Replace a matched `templateUrl` property's key/value in place. Returns
    /// the new value expression for the (now `template`) property.
    fn template_value(&mut self, url: &str, ast: AstBuilder<'a>) -> Expression<'a> {
        let normalized = normalize_url(url);
        if self.use_import {
            let var = format!("{RESOURCE_PREFIX}{}", self.counter);
            self.counter += 1;
            let arena_var = ast.allocator.alloc_str(&var);
            self.pending_imports.push((var, normalized));
            ast.expression_identifier(SPAN, arena_var)
        } else {
            let arena_src = ast.allocator.alloc_str(&normalized);
            let src = ast.expression_string_literal(SPAN, arena_src, None);
            let callee = ast.expression_identifier(SPAN, "require");
            let mut args = ast.vec();
            args.push(Argument::from(src));
            ast.expression_call(
                SPAN,
                callee,
                None::<TSTypeParameterInstantiation>,
                args,
                false,
            )
        }
    }

    /// Rewrite the `@Component({...})` metadata object: inline `templateUrl`,
    /// strip styles + `moduleId`.
    fn process_metadata(&mut self, obj: &mut ObjectExpression<'a>, ast: AstBuilder<'a>) {
        let old = std::mem::replace(&mut obj.properties, ast.vec());
        for prop in old {
            let ObjectPropertyKind::ObjectProperty(mut p) = prop else {
                obj.properties.push(prop);
                continue;
            };
            match key_name(&p.key) {
                Some("templateUrl") => {
                    if let Some(url_value) = static_string_value(&p.value) {
                        let value = self.template_value(&url_value, ast);
                        rewrite_to_template(&mut p, value, ast);
                        self.changed = true;
                    }
                    obj.properties.push(ObjectPropertyKind::ObjectProperty(p));
                }
                Some("styleUrls" | "styleUrl" | "styles" | "moduleId") => {
                    // Dropped entirely (styles are not exercised under test).
                    self.changed = true;
                }
                _ => obj.properties.push(ObjectPropertyKind::ObjectProperty(p)),
            }
        }
    }
}

/// The static string value of a `templateUrl` expression: a plain string literal
/// or a no-substitution template literal (`` `./x.html` ``). Anything dynamic
/// (interpolated template, identifier, …) returns `None` and is left un-inlined.
fn static_string_value(expr: &Expression<'_>) -> Option<String> {
    match expr {
        Expression::StringLiteral(s) => Some(s.value.as_str().to_string()),
        Expression::TemplateLiteral(t) if t.expressions.is_empty() && t.quasis.len() == 1 => t
            .quasis[0]
            .value
            .cooked
            .as_ref()
            .map(|c| c.as_str().to_string()),
        _ => None,
    }
}

/// Set `prop`'s key to `template` and value to `value`.
fn rewrite_to_template<'a>(
    prop: &mut ObjectProperty<'a>,
    value: Expression<'a>,
    ast: AstBuilder<'a>,
) {
    prop.key = PropertyKey::StaticIdentifier(ast.alloc_identifier_name(SPAN, "template"));
    prop.value = value;
}

impl<'a> Traverse<'a, ()> for ResourceTransform {
    fn enter_program(&mut self, node: &mut Program<'a>, _ctx: &mut TraverseCtx<'a, ()>) {
        for stmt in &node.body {
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
                    ImportDeclarationSpecifier::ImportSpecifier(s)
                        if s.imported.name().as_str() == "Component" =>
                    {
                        self.component_locals
                            .insert(s.local.name.as_str().to_string());
                    }
                    ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => {
                        self.component_namespaces
                            .insert(s.local.name.as_str().to_string());
                    }
                    _ => {}
                }
            }
        }
    }

    fn enter_class(&mut self, node: &mut Class<'a>, ctx: &mut TraverseCtx<'a, ()>) {
        if self.component_locals.is_empty() && self.component_namespaces.is_empty() {
            return;
        }
        let ast = ctx.ast;
        for dec in &mut node.decorators {
            if let Some(obj) =
                component_metadata(dec, &self.component_locals, &self.component_namespaces)
            {
                self.process_metadata(obj, ast);
            }
        }
    }

    fn exit_program(&mut self, node: &mut Program<'a>, ctx: &mut TraverseCtx<'a, ()>) {
        if self.pending_imports.is_empty() {
            return;
        }
        let ast = ctx.ast;
        let pending = std::mem::take(&mut self.pending_imports);
        for (index, (var, source)) in pending.into_iter().enumerate() {
            let arena_var = ast.allocator.alloc_str(&var);
            let arena_src = ast.allocator.alloc_str(&source);
            let local = ast.binding_identifier(SPAN, arena_var);
            let default_spec =
                ast.import_declaration_specifier_import_default_specifier(SPAN, local);
            let mut specs = ast.vec();
            specs.push(default_spec);
            let source_lit = ast.string_literal(SPAN, arena_src, None);
            let decl = ast.alloc_import_declaration(
                SPAN,
                Some(specs),
                source_lit,
                None,
                None::<WithClause>,
                ImportOrExportKind::Value,
            );
            node.body.insert(index, Statement::ImportDeclaration(decl));
        }
    }
}

/// If `dec` is `@Component({...})` resolving to an `@angular/core` `Component`,
/// return a mutable reference to its metadata object literal.
fn component_metadata<'b, 'a>(
    dec: &'b mut Decorator<'a>,
    component_locals: &HashSet<String>,
    component_namespaces: &HashSet<String>,
) -> Option<&'b mut ObjectExpression<'a>> {
    let Expression::CallExpression(call) = &mut dec.expression else {
        return None;
    };
    let is_component = match &call.callee {
        // `@Component(...)` — a named/aliased local.
        Expression::Identifier(id) => component_locals.contains(id.name.as_str()),
        // `@ng.Component(...)` — a namespace-qualified member.
        Expression::StaticMemberExpression(m) => {
            m.property.name.as_str() == "Component"
                && matches!(
                    &m.object,
                    Expression::Identifier(obj) if component_namespaces.contains(obj.name.as_str())
                )
        }
        _ => false,
    };
    if !is_component {
        return None;
    }
    match call.arguments.first_mut()? {
        Argument::ObjectExpression(obj) => Some(obj),
        _ => None,
    }
}
