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
