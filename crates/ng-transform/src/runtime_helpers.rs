//! Inline oxc's async runtime helpers into the emitted module (à la tsc's
//! inlined `__awaiter`).
//!
//! When `async`/`await` is downleveled (target < ES2017), oxc emits
//! `import _x from "@oxc-project/runtime/helpers/asyncToGenerator"` and the
//! helper builds the result Promise with `new Promise(...)`. As a *separate*
//! module, the helper's `Promise` can resolve to a different realm than the test
//! file's — under jest/vitest + jsdom + zone.js the test's global `Promise` is
//! replaced with `ZoneAwarePromise`, so the helper's Promise isn't
//! `instanceof`-equal (breaks `expect.any(Promise)` / `instanceof Promise`).
//!
//! tsc avoids this by *inlining* the helper, so its `new Promise` resolves to the
//! emitted module's (zone-patched) global. We reproduce that: replace the helper
//! import with a module-local `var <local> = <inline helper>` (emitted as prelude
//! text, like the interop helpers). Only the self-contained `asyncToGenerator`
//! (plain `async`/`await`) is inlined; the async-generator helpers are left as
//! runtime imports for now (rare in tests, and they depend on `OverloadYield`).

use oxc_allocator::Allocator;
use oxc_ast::AstBuilder;
use oxc_ast::ast::{ImportDeclarationSpecifier, Program, Statement};

const HELPER_PREFIX: &str = "@oxc-project/runtime/helpers/";

/// Inline expression returning the helper function — verbatim from
/// `@oxc-project/runtime`'s `asyncToGenerator`, wrapped in an IIFE. Its
/// `new Promise` / `Promise.resolve` resolve to the emitted module's global.
const ASYNC_TO_GENERATOR: &str = "(function () { function asyncGeneratorStep(n, t, e, r, o, a, c) { try { var i = n[a](c), u = i.value; } catch (n) { return void e(n); } i.done ? t(u) : Promise.resolve(u).then(r, o); } return function (n) { return function () { var t = this, e = arguments; return new Promise(function (r, o) { var a = n.apply(t, e); function _next(n) { asyncGeneratorStep(a, r, o, _next, _throw, \"next\", n); } function _throw(n) { asyncGeneratorStep(a, r, o, _next, _throw, \"throw\", n); } _next(void 0); }); }; }; })()";

/// `(helper file name, inline expression source)` for the helpers we inline.
const INLINE: &[(&str, &str)] = &[("asyncToGenerator", ASYNC_TO_GENERATOR)];

/// Drop `import <local> from "@oxc-project/runtime/helpers/<inlined>"` statements
/// and return a prelude (`var <local> = <inline>;`) the caller prepends before
/// the generated code. Empty when no such helper was imported (e.g. no
/// downleveled async, or a modern target).
#[must_use]
pub fn inline_async_helpers<'a>(allocator: &'a Allocator, program: &mut Program<'a>) -> String {
    let ast = AstBuilder::new(allocator);
    let mut prelude = String::new();
    let old = std::mem::replace(&mut program.body, ast.vec());
    for stmt in old {
        let inline_src = match &stmt {
            Statement::ImportDeclaration(import) => import
                .source
                .value
                .as_str()
                .strip_prefix(HELPER_PREFIX)
                .and_then(|name| INLINE.iter().find(|(n, _)| *n == name).map(|(_, s)| *s)),
            _ => None,
        };
        let Some(src) = inline_src else {
            program.body.push(stmt);
            continue;
        };
        // Emit `var <local> = <inline>;` for the (default) import binding(s).
        if let Statement::ImportDeclaration(import) = &stmt
            && let Some(specifiers) = &import.specifiers
        {
            for spec in specifiers {
                if let ImportDeclarationSpecifier::ImportDefaultSpecifier(d) = spec {
                    prelude.push_str("var ");
                    prelude.push_str(d.local.name.as_str());
                    prelude.push_str(" = ");
                    prelude.push_str(src);
                    prelude.push_str(";\n");
                }
            }
        }
        // Drop the import (replaced by the inlined prelude var).
    }
    prelude
}
