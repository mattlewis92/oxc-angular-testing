//! R16: oxc synthesizes a derived class's field-init constructor as
//! `constructor(..._args) { super(..._args); }`. Angular JIT's `isDelegateCtor`
//! only inherits the parent's DI params when the ctor delegates via
//! `super(...arguments)` (the tsc shape). Verify the post-pass rewrites it.

use ng_transform::{ModuleKind, TransformOptions, transform};

fn cjs_jit(src: &str) -> String {
    let opts = TransformOptions {
        module: ModuleKind::CommonJs,
        target: "es2016".into(),
        jit_transforms: true,
        ..Default::default()
    };
    transform(src, "m.ts", &opts).code
}

/// Stand-in for the reflection regex `@angular/core` uses to decide a derived
/// class inherits its parent's DI constructor params. Copied from
/// `@angular/core` **21.2.16**, `packages/core/src/reflection/reflection_capabilities.ts`:
///
/// ```text
/// INHERITED_CLASS_WITH_DELEGATE_CTOR =
///   /^class\s+[A-Za-z\d$_]*\s*extends\s+[^{]+{[\s\S]*constructor\s*\(\)\s*{[^}]*super\(\.\.\.arguments\)/
/// ```
///
/// (For function-form/ES5 output Angular uses a sibling `DELEGATE_CTOR` regex
/// keyed on `.apply(this, arguments)`; we emit ES `class` syntax, so the form
/// above is the relevant one.) The full regex also requires the enclosing
/// `class … extends …` context; here we collapse whitespace and check the
/// load-bearing `constructor(){super(...arguments)` adjacency. NOTE: this is a
/// hand-copied snapshot — Angular has changed this regex across majors, so
/// re-verify against the cited source on an Angular bump (a real-`@angular/core`
/// JIT canary would pin it; tracked separately).
fn matches_angular_delegate_ctor(code: &str) -> bool {
    let collapsed: String = code.split_whitespace().collect();
    collapsed.contains("constructor(){super(...arguments)")
}

#[test]
fn derived_synthesized_ctor_becomes_super_spread_arguments() {
    let code = cjs_jit(
        r#"import { Injectable } from '@angular/core';
@Injectable()
export class Child extends Base { x = 1; }
"#,
    );
    assert!(
        code.contains("constructor() {") && code.contains("super(...arguments);"),
        "{code}"
    );
    assert!(!code.contains("_args"), "rest param not removed: {code}");
    assert!(
        matches_angular_delegate_ctor(&code),
        "would not satisfy Angular isDelegateCtor: {code}"
    );
}

#[test]
fn oxc_synthesizes_the_expected_delegate_ctor_shape() {
    // Pre-rewrite CANARY against oxc itself. DelegateCtorTransform's matcher is keyed
    // to the exact shape oxc 0.126 synthesizes for a derived class's field-init
    // constructor: `constructor(...<ident>) { super(...<ident>); … }`. Run oxc's
    // transformer ALONE — the same env our pipeline uses (es2016 + set_public_class_fields)
    // and WITHOUT our delegate-ctor pass — and assert that shape. An oxc upgrade that
    // changes the shape (positional param before rest, `super.apply(this, arguments)`,
    // field-init moved to a helper, …) then fails HERE, naming oxc as the cause,
    // instead of letting delegate_ctor silently no-op and drop inherited DI params.
    use oxc_allocator::Allocator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_semantic::SemanticBuilder;
    use oxc_span::SourceType;
    use oxc_transformer::{CompilerAssumptions, EnvOptions, TransformOptions, Transformer};
    use std::path::Path;

    let allocator = Allocator::default();
    let source = "class Child extends Base { x = 1; }\n";
    let ret = Parser::new(&allocator, source, SourceType::ts()).parse();
    let mut program = ret.program;
    let scoping = SemanticBuilder::new()
        .build(&program)
        .semantic
        .into_scoping();
    let options = TransformOptions {
        env: EnvOptions::from_target("es2016").unwrap(),
        assumptions: CompilerAssumptions {
            set_public_class_fields: true, // mirrors useDefineForClassFields: false
            ..CompilerAssumptions::default()
        },
        ..TransformOptions::default()
    };
    Transformer::new(&allocator, Path::new("child.ts"), &options)
        .build_with_scoping(scoping, &mut program);
    let code = Codegen::new().build(&program).code;
    let collapsed: String = code.split_whitespace().collect();

    // A single rest param bound to a plain ident, delegating via `super(...<same ident>)`.
    // Name-agnostic (oxc may rename the rest) — only the SHAPE matters.
    let marker = "constructor(...";
    let start = collapsed.find(marker).unwrap_or_else(|| {
        panic!("oxc no longer synthesizes a rest-param delegate ctor — DelegateCtorTransform's matcher is stale:\n{code}")
    });
    let after = &collapsed[start + marker.len()..];
    let ident_end = after
        .find(')')
        .expect("malformed synthesized ctor param list");
    let ident = &after[..ident_end];
    assert!(
        !ident.is_empty()
            && ident
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '$'),
        "rest binding must be a single plain identifier (got {ident:?}):\n{code}"
    );
    assert!(
        collapsed.contains(&format!("super(...{ident})")),
        "synthesized ctor must delegate via super(...{ident}):\n{code}"
    );
}

#[test]
fn derived_class_without_fields_synthesizes_no_constructor() {
    // Pins the "when does oxc synthesize a delegate ctor" assumption: with NO class
    // fields (nothing to host) and no explicit ctor, oxc synthesizes no constructor
    // at all, so delegate_ctor correctly no-ops and Angular inherits DI params via its
    // INHERITED_CLASS path (not the delegate-ctor path). If oxc ever started emitting a
    // ctor here, this fails and we re-examine the rewrite's scope.
    let code = cjs_jit(
        r#"import { Injectable } from '@angular/core';
@Injectable()
export class Child extends Base {}
"#,
    );
    assert!(
        !code.contains("constructor"),
        "no constructor should be synthesized for a field-less derived class: {code}"
    );
}

#[test]
fn explicit_ctor_is_unchanged() {
    let code = cjs_jit(
        r#"import { Injectable } from '@angular/core';
@Injectable()
export class Child extends Base { x = 1; constructor() { super(); this.y = 2; } }
"#,
    );
    // super() stays a no-arg call; no `...arguments` injected.
    assert!(code.contains("super();"), "{code}");
    assert!(!code.contains("super(...arguments)"), "{code}");
}

#[test]
fn hand_written_rest_ctor_that_uses_args_is_not_rewritten() {
    // The shape matches (rest param + `super(...args)` first), but `args` is used
    // again after super — rewriting to `super(...arguments)` would drop the param
    // and leave `this.extra = args.length` dangling. The reference-count guard must
    // bail (a synthesized delegate ctor never references the rest beyond super).
    let code = cjs_jit(
        r#"export class Child extends Base {
  constructor(...args) { super(...args); this.extra = args.length; }
}
"#,
    );
    assert!(
        code.contains("super(...args)") && !code.contains("super(...arguments)"),
        "rest ctor that reuses args must be left intact: {code}"
    );
    assert!(code.contains("args.length"), "{code}");
}

#[test]
fn non_extends_class_is_unchanged() {
    let code = cjs_jit(
        r#"import { Injectable } from '@angular/core';
@Injectable()
export class Child { x = 1; }
"#,
    );
    assert!(code.contains("constructor() {"), "{code}");
    assert!(
        !code.contains("super"),
        "no super in a non-derived class: {code}"
    );
}
