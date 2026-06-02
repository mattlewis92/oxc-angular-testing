//! Port of `babel-plugin-jest-hoist`: hoist `jest.mock()` and friends to the top
//! of their containing block, above imports.
//!
//! `jest.mock('./x', factory)` must be registered *before* the module under test
//! is loaded. Users write it after their imports, so jest's babel transform
//! hoists it. We replace babel-jest, so we reproduce the hoist here.
//!
//! Hoisted methods (same set as babel): `mock`, `unmock`, `deepUnmock`,
//! `enableAutomock`, `disableAutomock`. `doMock` / `dontMock` are intentionally
//! **not** hoisted (their whole point is to run in place). The `jest` object is
//! the global, an `import { jest } from '@jest/globals'` binding, or a
//! `const { jest } = require('@jest/globals')` binding.
//!
//! Only the `jest.*` *call* is hoisted — the mock factory is a lazy closure,
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

const JEST_GLOBALS: &str = "@jest/globals";
const HOISTED_METHODS: &[&str] = &[
    "mock",
    "unmock",
    "deepUnmock",
    "enableAutomock",
    "disableAutomock",
];

pub struct JestHoist {
    /// Local identifier names that refer to `jest` — the global plus any
    /// `@jest/globals` import/require aliases.
    jest_locals: HashSet<String>,
    pub changed: bool,
}

impl JestHoist {
    #[must_use]
    pub fn new() -> Self {
        let mut jest_locals = HashSet::new();
        jest_locals.insert("jest".to_string());
        Self {
            jest_locals,
            changed: false,
        }
    }
}

impl Default for JestHoist {
    fn default() -> Self {
        Self::new()
    }
}

/// Is `stmt` a hoistable `jest.<method>(...)` expression statement?
fn is_hoistable(stmt: &Statement<'_>, jest_locals: &HashSet<String>) -> bool {
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
    jest_locals.contains(obj.name.as_str())
        && HOISTED_METHODS.contains(&member.property.name.as_str())
}

impl<'a> Traverse<'a, ()> for JestHoist {
    fn enter_program(&mut self, node: &mut Program<'a>, _ctx: &mut TraverseCtx<'a, ()>) {
        for stmt in &node.body {
            match stmt {
                // import { jest } from '@jest/globals'  (incl. `jest as alias`)
                Statement::ImportDeclaration(import)
                    if import.source.value.as_str() == JEST_GLOBALS =>
                {
                    let Some(specifiers) = &import.specifiers else {
                        continue;
                    };
                    for spec in specifiers {
                        if let ImportDeclarationSpecifier::ImportSpecifier(s) = spec
                            && s.imported.name().as_str() == "jest"
                        {
                            self.jest_locals.insert(s.local.name.as_str().to_string());
                        }
                    }
                }
                // const { jest } = require('@jest/globals')  (incl. `jest: alias`)
                Statement::VariableDeclaration(decl) => {
                    for d in &decl.declarations {
                        if let Some(local) = jest_destructured_from_require(d) {
                            self.jest_locals.insert(local);
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
        if !stmts.iter().any(|s| is_hoistable(s, &self.jest_locals)) {
            return;
        }
        let ast: AstBuilder<'a> = ctx.ast;
        let old = std::mem::replace(stmts, ast.vec());
        let mut hoisted = ast.vec();
        let mut rest = ast.vec();
        for s in old {
            if is_hoistable(&s, &self.jest_locals) {
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

/// If `decl` is `{ jest } = require('@jest/globals')` (or `{ jest: alias }`),
/// return the local binding name.
fn jest_destructured_from_require(decl: &VariableDeclarator<'_>) -> Option<String> {
    // init must be `require('@jest/globals')`
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
    if arg.value.as_str() != JEST_GLOBALS {
        return None;
    }
    // id must be an object pattern with a `jest` property.
    let BindingPattern::ObjectPattern(obj) = &decl.id else {
        return None;
    };
    for prop in &obj.properties {
        if let PropertyKey::StaticIdentifier(key) = &prop.key
            && key.name == "jest"
            && let BindingPattern::BindingIdentifier(local) = &prop.value
        {
            return Some(local.name.as_str().to_string());
        }
    }
    None
}
