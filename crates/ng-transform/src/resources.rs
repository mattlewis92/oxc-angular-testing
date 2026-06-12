//! `replaceResources` pass — the jest-preset-angular component resource
//! transform, ported to an oxc [`Traverse`] pass.
//!
//! For classes decorated with `@Component({...})` (where `Component` resolves to
//! an `@angular/core` import):
//!
//! - `templateUrl: './x.html'` → `template: require('./x.html')` (require mode)
//!   or a hoisted `import __NG_CLI_RESOURCE__N from './x.html'` + `template:
//!   __NG_CLI_RESOURCE__N` (import mode).
//! - `styleUrls`, `styleUrl`, `styles`, `moduleId` → removed (default), or —
//!   with `keep_styles` — style URLs are rewritten to imports/requires (with an
//!   optional query parameter, e.g. vite's `?inline`, so the bundler compiles
//!   the CSS and yields a string) and merged with any inline `styles` into a
//!   single `styles: [...]` (inline entries first, URL entries after, matching
//!   Angular's `resolveComponentResources` order). `moduleId` is still removed.
//!
//! Detection of the `@angular/core` origin is by import tracking (named
//! specifiers, including aliases). The TypeScript original uses the type
//! checker; this is the lightweight equivalent.

use std::collections::HashSet;

use oxc_ast::AstBuilder;
use oxc_ast::ast::{
    Argument, ArrayExpressionElement, Class, Decorator, Expression, ImportDeclarationSpecifier,
    ImportOrExportKind, ObjectExpression, ObjectProperty, ObjectPropertyKind, Program, PropertyKey,
    Statement, TSTypeParameterInstantiation, WithClause,
};
use oxc_span::SPAN;
use oxc_traverse::{Traverse, TraverseCtx};

const RESOURCE_PREFIX: &str = "__NG_CLI_RESOURCE__";
const STYLE_PREFIX: &str = "__oxc_ng_style_";
const ANGULAR_CORE: &str = "@angular/core";

/// State for the resource transform.
pub struct ResourceTransform {
    /// Emit a hoisted top-level `import` instead of `require(...)`.
    use_import: bool,
    /// Keep styles: rewrite `styleUrl(s)` to imports/requires instead of
    /// stripping all style metadata.
    keep_styles: bool,
    /// Query parameter appended to each rewritten style URL (e.g. `inline` →
    /// `./a.scss?inline`, or `&inline` when the URL already has a query), so a
    /// bundler can return the compiled CSS as a string. `None` emits the URL
    /// verbatim.
    style_query: Option<String>,
    /// Local identifier names that refer to `Component` from `@angular/core`.
    component_locals: HashSet<String>,
    /// Local names of `import * as ng from '@angular/core'` namespace imports, so
    /// `@ng.Component({ templateUrl })` is recognized too (parity with jit_transform).
    component_namespaces: HashSet<String>,
    /// `import` mode: collected `(local_var, normalized_source)` to hoist.
    pending_imports: Vec<(String, String)>,
    /// Counter for `__NG_CLI_RESOURCE__N` names.
    counter: usize,
    /// Counter for `__oxc_ng_style_N__` names (shared across all components in
    /// the file, so naming is stable and collision-free within the module).
    style_counter: usize,
    /// Prefix for hoisted style identifiers. Starts as [`STYLE_PREFIX`]; if the
    /// source text already contains that prefix anywhere (a user binding could
    /// collide), underscores are prepended until it no longer occurs — cheap,
    /// conservative, and deterministic (same input → same names).
    style_prefix: String,
    /// Whether this pass changed anything (resources or imports).
    pub changed: bool,
}

impl ResourceTransform {
    #[must_use]
    pub fn new(
        use_import: bool,
        keep_styles: bool,
        style_query: Option<String>,
        source: &str,
    ) -> Self {
        let mut style_prefix = STYLE_PREFIX.to_string();
        while source.contains(&style_prefix) {
            style_prefix.insert(0, '_');
        }
        Self {
            use_import,
            keep_styles,
            style_query,
            component_locals: HashSet::new(),
            component_namespaces: HashSet::new(),
            pending_imports: Vec::new(),
            counter: 0,
            style_counter: 0,
            style_prefix,
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

    /// The expression a single style URL becomes under `keep_styles`: the URL is
    /// normalized, given the configured query parameter (if any — e.g. `inline`
    /// → `?inline`, or `&inline` if the URL already has a query), and turned
    /// into a hoisted default import's identifier (import mode) or an in-place
    /// `require(...)` call (require mode). The bundler's CSS pipeline compiles
    /// the stylesheet; a query like vite's `inline` makes the default export
    /// the CSS text.
    fn style_value(&mut self, url: &str, ast: AstBuilder<'a>) -> Expression<'a> {
        let mut specifier = normalize_url(url);
        if let Some(query) = &self.style_query {
            specifier.push(if specifier.contains('?') { '&' } else { '?' });
            specifier.push_str(query);
        }
        if self.use_import {
            let var = format!("{}{}__", self.style_prefix, self.style_counter);
            self.style_counter += 1;
            let arena_var = ast.allocator.alloc_str(&var);
            self.pending_imports.push((var, specifier));
            ast.expression_identifier(SPAN, arena_var)
        } else {
            let arena_src = ast.allocator.alloc_str(&specifier);
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
    /// strip `moduleId`, and strip (default) or rewrite (`keep_styles`) the
    /// style metadata.
    fn process_metadata(&mut self, obj: &mut ObjectExpression<'a>, ast: AstBuilder<'a>) {
        // `keep_styles`: collect the static style URLs up front. If any style URL
        // is dynamic — or inline `styles` has a shape we cannot merge into — the
        // style properties are left untouched (same policy as a dynamic
        // `templateUrl`), since a partial rewrite would change behavior.
        // `None` = strip (the keep_styles: false default); `Some(plan)` = keep.
        let style_plan = self.keep_styles.then(|| plan_styles(obj));
        // URL-derived style expressions, in declaration order. Built before the
        // rebuild loop so the merged `styles` array can land at the position of
        // the first style-related property.
        let url_styles: Vec<Expression<'a>> = match &style_plan {
            Some(StylePlan::Rewrite { urls }) => {
                urls.iter().map(|u| self.style_value(u, ast)).collect()
            }
            _ => Vec::new(),
        };

        // Index (in the rebuilt property list) of the first style-related
        // property, and the inline `styles` elements captured during the rebuild.
        let mut styles_slot: Option<usize> = None;
        let mut inline_elements = ast.vec::<ArrayExpressionElement<'a>>();

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
                Some("moduleId") => {
                    // Dropped in both modes (JIT has no module-relative loading).
                    self.changed = true;
                }
                Some(key @ ("styleUrls" | "styleUrl" | "styles")) => match &style_plan {
                    None => {
                        // Default: dropped entirely (styles are not exercised
                        // under jsdom-style tests).
                        self.changed = true;
                    }
                    Some(StylePlan::Untouched | StylePlan::InlineOnly) => {
                        // Untouched: a dynamic URL / un-mergeable inline shape —
                        // leave every style property as written. InlineOnly:
                        // nothing to rewrite, inline `styles` pass through.
                        obj.properties.push(ObjectPropertyKind::ObjectProperty(p));
                    }
                    Some(StylePlan::Rewrite { .. }) => {
                        styles_slot.get_or_insert(obj.properties.len());
                        if key == "styles" {
                            // Take ownership of the value (the property itself is
                            // dropped) so its elements can be spliced through.
                            let value =
                                std::mem::replace(&mut p.value, ast.expression_null_literal(SPAN));
                            collect_inline_elements(value, &mut inline_elements, ast);
                        }
                        self.changed = true;
                    }
                },
                _ => obj.properties.push(ObjectPropertyKind::ObjectProperty(p)),
            }
        }

        if let Some(slot) = styles_slot {
            // Merged `styles: [...]`: inline entries first, URL-derived entries
            // after — Angular's `resolveComponentResources` appends fetched
            // styleUrl styles to the end of the inline `styles` array.
            inline_elements.extend(url_styles.into_iter().map(ArrayExpressionElement::from));
            let array = ast.expression_array(SPAN, inline_elements);
            let key = PropertyKey::StaticIdentifier(ast.alloc_identifier_name(SPAN, "styles"));
            let prop = ast.object_property_kind_object_property(
                SPAN,
                oxc_ast::ast::PropertyKind::Init,
                key,
                array,
                false,
                false,
                false,
            );
            obj.properties.insert(slot, prop);
        }
    }
}

/// How `keep_styles` will treat a component's style metadata.
enum StylePlan {
    /// Inline `styles` only (no `styleUrl`/`styleUrls`): preserved verbatim.
    InlineOnly,
    /// Rewrite: replace the style properties with one merged `styles: [...]`
    /// whose URL-derived entries come from these static URLs.
    Rewrite { urls: Vec<String> },
    /// A dynamic style URL or un-mergeable inline `styles` shape: leave every
    /// style property untouched.
    Untouched,
}

/// Decide the [`StylePlan`] for a metadata object (immutable pre-pass).
fn plan_styles(obj: &ObjectExpression<'_>) -> StylePlan {
    let mut urls: Vec<String> = Vec::new();
    let mut has_urls = false;
    let mut inline_mergeable = true;
    for prop in &obj.properties {
        let ObjectPropertyKind::ObjectProperty(p) = prop else {
            continue;
        };
        match key_name(&p.key) {
            Some("styleUrl") => {
                has_urls = true;
                match static_string_value(&p.value) {
                    Some(url) => urls.push(url),
                    None => return StylePlan::Untouched,
                }
            }
            Some("styleUrls") => {
                has_urls = true;
                let Expression::ArrayExpression(arr) = &p.value else {
                    return StylePlan::Untouched;
                };
                for element in &arr.elements {
                    match element.as_expression().and_then(static_string_value) {
                        Some(url) => urls.push(url),
                        None => return StylePlan::Untouched,
                    }
                }
            }
            Some("styles") => {
                // Mergeable shapes: an array literal (its elements are spliced
                // through) or a single string/template literal (Angular accepts
                // `styles: '…'` and normalizes it to a one-element array).
                inline_mergeable = matches!(
                    &p.value,
                    Expression::ArrayExpression(_)
                        | Expression::StringLiteral(_)
                        | Expression::TemplateLiteral(_)
                );
            }
            _ => {}
        }
    }
    if !has_urls {
        return StylePlan::InlineOnly;
    }
    if !inline_mergeable {
        return StylePlan::Untouched;
    }
    StylePlan::Rewrite { urls }
}

/// Splice an inline `styles` value into `elements`: array literals contribute
/// their elements (spreads included), a single string/template literal becomes
/// one element (Angular's `styles: '…'` normalization).
fn collect_inline_elements<'a>(
    value: Expression<'a>,
    elements: &mut oxc_allocator::Vec<'a, ArrayExpressionElement<'a>>,
    ast: AstBuilder<'a>,
) {
    match value {
        Expression::ArrayExpression(mut arr) => {
            let inner = std::mem::replace(&mut arr.elements, ast.vec());
            elements.extend(inner);
        }
        other => elements.push(ArrayExpressionElement::from(other)),
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
