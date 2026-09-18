//! tsconfig `strictNullChecks: false` decorator-metadata parity.
//!
//! TypeScript's `emitDecoratorMetadata` serializer skips `null`/`undefined`
//! union constituents when `strictNullChecks` is OFF: `string | null`
//! serializes as `String`, where strict mode yields `Object` (TS
//! `serializeUnionOrIntersectionType`). oxc's serializer (upstream
//! `oxc_transformer`) implements only the strict behavior, so for an SNC-off
//! project we reproduce tsc by rewriting the type ANNOTATIONS before lowering:
//! drop `null`/`undefined` from every union, collapsing a single survivor to a
//! bare type. Types are erased at emit, so the rewrite is observable only
//! through metadata serialization — exactly the intended effect.

use oxc_ast::ast::TSType;
use oxc_ast_visit::{VisitMut, walk_mut};

pub struct StripNullableUnions;

fn is_nullable(t: &TSType<'_>) -> bool {
    matches!(t, TSType::TSNullKeyword(_) | TSType::TSUndefinedKeyword(_))
}

impl<'a> VisitMut<'a> for StripNullableUnions {
    fn visit_ts_type(&mut self, ty: &mut TSType<'a>) {
        if let TSType::TSUnionType(union) = ty {
            let keep = union.types.iter().filter(|t| !is_nullable(t)).count();
            // All-nullable (`null | undefined`) stays a union — both serialize
            // to Object either way, and an empty union isn't a valid type.
            if keep > 0 && keep < union.types.len() {
                union.types.retain(|t| !is_nullable(t));
                if keep == 1 {
                    if let Some(single) = union.types.pop() {
                        *ty = single;
                    }
                }
            }
        }
        walk_mut::walk_ts_type(self, ty);
    }
}
