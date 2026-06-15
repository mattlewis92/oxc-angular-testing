//! Port of `babel-plugin-jest-hoist`: hoist a test runner's mock-registration
//! calls (`mock()` / `unmock()` / …) to the top of their containing block, above
//! imports.
//!
//! `jest.mock('./x', factory)` (and the vitest equivalent `vi.mock`) must be
//! registered *before* the module under test is loaded. Users write it after
//! their imports, so the runner's babel transform hoists it; we replace that
//! transform, so we reproduce the hoist here. Which runner — and thus the object
//! name (`jest` / `vi`), the import module (`@jest/globals` / `vitest`), and the
//! hoisted method set — is selected by [`MockFramework`].
//!
//! Hoisted methods:
//! - jest (same set as babel): `mock`, `unmock`, `deepUnmock`, `enableAutomock`,
//!   `disableAutomock`.
//! - vitest: `mock`, `unmock`.
//!
//! `doMock` / `dontMock` (jest) and `doMock` / `doUnmock` (vitest) are
//! intentionally **not** hoisted — their whole point is to run in place. The
//! runner object is the global, an `import { jest } from '@jest/globals'` /
//! `import { vi } from 'vitest'` binding, or a `const { jest } = require(...)`
//! binding.
//!
//! Only the `<obj>.*` *call* is hoisted — the mock factory is a lazy closure,
//! invoked when the mocked module is required (after the hoisted call), so
//! `mock`-prefixed locals it references don't need hoisting too. (babel's
//! out-of-scope factory *validation* is a developer guard, not required for
//! correctness, and is intentionally not ported here.)

use std::collections::HashSet;

use oxc_allocator::Vec as ArenaVec;
use oxc_ast::AstBuilder;
use oxc_ast::ast::{
    Argument, BindingPattern, Expression, ImportDeclarationSpecifier, Program, PropertyKey,
    Statement, VariableDeclarator,
};
use oxc_traverse::{Traverse, TraverseCtx};

pub use crate::options::MockFramework;

impl MockFramework {
    /// The runner object name and its default global binding (`jest` / `vi`).
    fn object_name(self) -> &'static str {
        match self {
            MockFramework::Jest => "jest",
            MockFramework::Vi => "vi",
        }
    }

    /// The module the runner object is imported from / required of.
    fn module_source(self) -> &'static str {
        match self {
            MockFramework::Jest => "@jest/globals",
            MockFramework::Vi => "vitest",
        }
    }

    /// Methods hoisted above imports for this runner.
    fn hoisted_methods(self) -> &'static [&'static str] {
        match self {
            MockFramework::Jest => &[
                "mock",
                "unmock",
                "deepUnmock",
                "enableAutomock",
                "disableAutomock",
            ],
            MockFramework::Vi => &["mock", "unmock"],
        }
    }
}

pub struct MockHoist {
    framework: MockFramework,
    /// Local identifier names that refer to the runner object — the global plus
    /// any module import/require aliases.
    locals: HashSet<String>,
    pub changed: bool,
}

impl MockHoist {
    #[must_use]
    pub fn new(framework: MockFramework) -> Self {
        let mut locals = HashSet::new();
        locals.insert(framework.object_name().to_string());
        Self {
            framework,
            locals,
            changed: false,
        }
    }
}

/// Is `stmt` a hoistable `<obj>.<method>(...)` expression statement?
fn is_hoistable(stmt: &Statement<'_>, locals: &HashSet<String>, methods: &[&str]) -> bool {
    let Statement::ExpressionStatement(es) = stmt else {
        return false;
    };
    let Expression::CallExpression(call) = &es.expression else {
        return false;
    };
    let Expression::StaticMemberExpression(member) = &call.callee else {
        return false;
    };
    let Expression::Identifier(obj) = &member.object else {
        return false;
    };
    locals.contains(obj.name.as_str()) && methods.contains(&member.property.name.as_str())
}

impl<'a> Traverse<'a, ()> for MockHoist {
    fn enter_program(&mut self, node: &mut Program<'a>, _ctx: &mut TraverseCtx<'a, ()>) {
        let module_source = self.framework.module_source();
        let object_name = self.framework.object_name();
        for stmt in &node.body {
            match stmt {
                // import { jest } from '@jest/globals'  (incl. `jest as alias`)
                Statement::ImportDeclaration(import)
                    if import.source.value.as_str() == module_source =>
                {
                    let Some(specifiers) = &import.specifiers else {
                        continue;
                    };
                    for spec in specifiers {
                        if let ImportDeclarationSpecifier::ImportSpecifier(s) = spec
                            && s.imported.name().as_str() == object_name
                        {
                            self.locals.insert(s.local.name.as_str().to_string());
                        }
                    }
                }
                // const { jest } = require('@jest/globals')  (incl. `jest: alias`)
                Statement::VariableDeclaration(decl) => {
                    for d in &decl.declarations {
                        if let Some(local) =
                            object_destructured_from_require(d, module_source, object_name)
                        {
                            self.locals.insert(local);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // Hoist on exit so nested blocks are processed first; fires for every
    // statement list (program, function/arrow body, block).
    fn exit_statements(
        &mut self,
        stmts: &mut ArenaVec<'a, Statement<'a>>,
        ctx: &mut TraverseCtx<'a, ()>,
    ) {
        let methods = self.framework.hoisted_methods();
        if !stmts.iter().any(|s| is_hoistable(s, &self.locals, methods)) {
            return;
        }
        let ast: AstBuilder<'a> = ctx.ast;
        let old = std::mem::replace(stmts, ast.vec());
        let mut hoisted = ast.vec();
        let mut rest = ast.vec();
        for s in old {
            if is_hoistable(&s, &self.locals, methods) {
                hoisted.push(s);
            } else {
                rest.push(s);
            }
        }
        // Hoisted calls first (in source order), then the rest (in source order).
        for s in rest {
            hoisted.push(s);
        }
        *stmts = hoisted;
        self.changed = true;
    }
}

/// If `decl` is `{ <object_name> } = require('<module_source>')` (or
/// `{ <object_name>: alias }`), return the local binding name.
fn object_destructured_from_require(
    decl: &VariableDeclarator<'_>,
    module_source: &str,
    object_name: &str,
) -> Option<String> {
    // init must be `require('<module_source>')`
    let Expression::CallExpression(call) = decl.init.as_ref()? else {
        return None;
    };
    let Expression::Identifier(callee) = &call.callee else {
        return None;
    };
    if callee.name != "require" {
        return None;
    }
    let Some(Argument::StringLiteral(arg)) = call.arguments.first() else {
        return None;
    };
    if arg.value.as_str() != module_source {
        return None;
    }
    // id must be an object pattern with an `<object_name>` property.
    let BindingPattern::ObjectPattern(obj) = &decl.id else {
        return None;
    };
    for prop in &obj.properties {
        if let PropertyKey::StaticIdentifier(key) = &prop.key
            && key.name == object_name
            && let BindingPattern::BindingIdentifier(local) = &prop.value
        {
            return Some(local.name.as_str().to_string());
        }
    }
    None
}
