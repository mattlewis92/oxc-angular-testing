//! Same-file enum-member reference folding (tsc parity).
//!
//! `enum B { X = A.Y }` where `A` is a string enum in the same file: tsc's
//! constant evaluator folds the reference to `A.Y`'s literal, so the member
//! emits in the STRING-enum form (`B["X"] = "y"` — no reverse mapping). oxc's
//! enum lowering leaves the reference as a runtime expression and emits the
//! NUMERIC-enum form `B[B["X"] = A.Y] = "X"`, adding a bogus reverse mapping
//! for the string value — `Object.values(B)` then contains phantom member
//! names, breaking every consumer that enumerates the enum (e.g.
//! `it.each(Object.values(E))`). Fold such references to literals up front so
//! oxc sees plain strings.
//!
//! Only STRING values fold: a reference resolving to a number emits the
//! numeric-enum form either way, where the reverse mapping is correct.
//! Cross-file references are left alone — per-file transpile (tsc
//! `transpileModule` / ts-jest) cannot fold those either, so leaving them
//! matches the baseline emit.

use std::collections::HashMap;

use oxc_ast::AstBuilder;
use oxc_ast::ast::{
    Declaration, Expression, Program, Statement, TSEnumDeclaration, TSEnumMemberName,
};
use oxc_span::GetSpan;

#[derive(Clone, Debug)]
enum EnumValue {
    Str(String),
    Num(f64),
}

pub fn fold_same_file_enum_refs<'a>(program: &mut Program<'a>, ast: AstBuilder<'a>) {
    // enum name → member name → literal value, populated in source order: a
    // reference folds only when the referenced enum is declared ABOVE it —
    // matching runtime evaluation order (forward references throw at runtime,
    // and tsc's checker rejects them).
    let mut known: HashMap<String, HashMap<String, EnumValue>> = HashMap::new();
    for stmt in &mut program.body {
        let decl = match stmt {
            Statement::TSEnumDeclaration(d) => &mut **d,
            Statement::ExportNamedDeclaration(e) => match &mut e.declaration {
                Some(Declaration::TSEnumDeclaration(d)) => &mut **d,
                _ => continue,
            },
            _ => continue,
        };
        if !decl.declare {
            fold_enum(decl, &mut known, ast);
        }
    }
}

fn member_name(id: &TSEnumMemberName) -> Option<String> {
    match id {
        TSEnumMemberName::Identifier(n) => Some(n.name.to_string()),
        TSEnumMemberName::String(s) | TSEnumMemberName::ComputedString(s) => {
            Some(s.value.to_string())
        }
        TSEnumMemberName::ComputedTemplateString(_) => None,
    }
}

fn fold_enum<'a>(
    decl: &mut TSEnumDeclaration<'a>,
    known: &mut HashMap<String, HashMap<String, EnumValue>>,
    ast: AstBuilder<'a>,
) {
    let mut values: HashMap<String, EnumValue> = HashMap::new();
    // Next auto-increment value; `None` once a member's value is unknowable
    // (tsc would then require initializers on the following members anyway).
    let mut auto: Option<f64> = Some(0.0);
    for member in &mut decl.body.members {
        if let Some(init) = &member.initializer
            && let Some(EnumValue::Str(s)) = resolve_ref(init, known)
        {
            let span = init.span();
            member.initializer =
                Some(ast.expression_string_literal(span, ast.allocator.alloc_str(&s), None));
        }
        let value = match &member.initializer {
            None => auto.map(EnumValue::Num),
            Some(Expression::StringLiteral(s)) => Some(EnumValue::Str(s.value.to_string())),
            Some(Expression::NumericLiteral(n)) => Some(EnumValue::Num(n.value)),
            Some(_) => None,
        };
        auto = match &value {
            Some(EnumValue::Num(n)) => Some(n + 1.0),
            _ => None,
        };
        if let (Some(name), Some(v)) = (member_name(&member.id), value) {
            values.insert(name, v);
        }
    }
    known.insert(decl.id.name.to_string(), values);
}

/// Resolve `EnumName.MEMBER` / `EnumName["MEMBER"]` against the same-file
/// enums collected so far.
fn resolve_ref(
    expr: &Expression<'_>,
    known: &HashMap<String, HashMap<String, EnumValue>>,
) -> Option<EnumValue> {
    match expr {
        Expression::StaticMemberExpression(m) => {
            let Expression::Identifier(obj) = &m.object else {
                return None;
            };
            known
                .get(obj.name.as_str())?
                .get(m.property.name.as_str())
                .cloned()
        }
        Expression::ComputedMemberExpression(m) => {
            let Expression::Identifier(obj) = &m.object else {
                return None;
            };
            let Expression::StringLiteral(key) = &m.expression else {
                return None;
            };
            known
                .get(obj.name.as_str())?
                .get(key.value.as_str())
                .cloned()
        }
        _ => None,
    }
}
