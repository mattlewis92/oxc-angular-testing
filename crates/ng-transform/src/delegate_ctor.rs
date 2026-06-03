//! Rewrite oxc's synthesized derived-class constructor to match tsc.
//!
//! When a class with class fields `extends` a base and has no explicit
//! constructor, oxc's class-field lowering synthesizes a delegating
//! constructor so the field initializers have somewhere to live:
//!
//! ```js
//! constructor(..._args) { super(..._args); this.x = 1; }
//! ```
//!
//! TypeScript emits the same thing as:
//!
//! ```js
//! constructor() { super(...arguments); this.x = 1; }
//! ```
//!
//! Angular JIT's `isDelegateCtor` (in `@angular/core`'s reflection) only
//! recognizes the *latter* shape — an empty parameter list followed by
//! `super(...arguments)` — and only then inherits the parent's DI
//! constructor parameters. The `(..._args) => super(..._args)` form fails
//! that regex, so a `@Injectable()` derived service silently loses its
//! inherited dependencies.
//!
//! This pass runs **after** oxc's transformer (so it sees the lowered,
//! synthesized constructor) and rewrites exactly that shape back to the tsc
//! form. The match is intentionally narrow:
//!
//! - the class must `extends` something,
//! - the constructor's parameter list must be *only* a single rest element
//!   bound to a plain identifier (the synthesized `_args`),
//! - the first body statement must be `super(...<that same identifier>)`,
//! - and the rest binding is referenced *exactly once* (that `super` spread).
//!
//! The last condition guards hand-written code: a `constructor(...args) {
//! super(...args); }` with no further use of `args` rewrites safely (the rest
//! carries exactly what `arguments` does, with no reassignment before `super`),
//! but a `constructor(...args) { super(...args); use(args); }` is left untouched —
//! dropping the param would leave `use(args)` dangling. The synthesized delegate
//! ctor only ever references the rest in the `super` call, so it always matches.

use oxc_ast::ast::{Argument, Class, ClassElement, Expression, Statement};
use oxc_span::SPAN;
use oxc_traverse::{Traverse, TraverseCtx};

pub struct DelegateCtorTransform;

impl DelegateCtorTransform {
    pub fn new() -> Self {
        Self
    }
}

impl<'a> Traverse<'a, ()> for DelegateCtorTransform {
    fn enter_class(&mut self, class: &mut Class<'a>, ctx: &mut TraverseCtx<'a, ()>) {
        // Only derived classes synthesize a delegating constructor.
        if class.super_class.is_none() {
            return;
        }
        for element in &mut class.body.body {
            let ClassElement::MethodDefinition(method) = element else {
                continue;
            };
            if !method.kind.is_constructor() {
                continue;
            }
            let func = &mut method.value;

            // The parameter list must be exactly `(...<ident>)`: no leading
            // positional params, and a rest element bound to a plain identifier.
            if !func.params.items.is_empty() {
                continue;
            }
            let Some((rest_name, rest_symbol)) = func
                .params
                .rest
                .as_ref()
                .and_then(|rest| rest.rest.argument.get_binding_identifier())
                .and_then(|ident| {
                    ident
                        .symbol_id
                        .get()
                        .map(|sym| (ident.name.to_string(), sym))
                })
            else {
                continue;
            };

            // The first statement must be `super(...<rest_name>)`.
            let is_delegate = func
                .body
                .as_deref()
                .and_then(|body| body.statements.first())
                .is_some_and(|first| is_super_spread_of(first, &rest_name));
            if !is_delegate {
                continue;
            }

            // Conservative guard: only rewrite when the rest binding is referenced
            // EXACTLY once — the `super(...rest)` spread we're about to replace. The
            // synthesized delegate ctor references `_args` nowhere else, so this never
            // blocks the real target; but a hand-written `constructor(...a){
            // super(...a); use(a); }` (which also matches the shape) would otherwise
            // miscompile — dropping the param leaves `use(a)` dangling.
            if ctx.scoping().get_resolved_references(rest_symbol).count() != 1 {
                continue;
            }

            // Rewrite: drop the rest param, and change the super call to
            // `super(...arguments)`.
            func.params.rest = None;
            if let Some(body) = func.body.as_deref_mut() {
                if let Some(Statement::ExpressionStatement(stmt)) = body.statements.first_mut() {
                    if let Expression::CallExpression(call) = &mut stmt.expression {
                        let args_ident = ctx.ast.expression_identifier(SPAN, "arguments");
                        call.arguments.clear();
                        call.arguments
                            .push(ctx.ast.argument_spread_element(SPAN, args_ident));
                    }
                }
            }
        }
    }
}

/// `true` if `stmt` is an expression statement `super(...<name>)` — a `super`
/// call whose sole argument is a spread of the identifier `name`.
fn is_super_spread_of(stmt: &Statement, name: &str) -> bool {
    let Statement::ExpressionStatement(stmt) = stmt else {
        return false;
    };
    let Expression::CallExpression(call) = &stmt.expression else {
        return false;
    };
    if !matches!(call.callee, Expression::Super(_)) {
        return false;
    }
    if call.arguments.len() != 1 {
        return false;
    }
    let Argument::SpreadElement(spread) = &call.arguments[0] else {
        return false;
    };
    matches!(&spread.argument, Expression::Identifier(id) if id.name.as_str() == name)
}
