//! Verifies TS→JS + decorator lowering produces executable JavaScript with the
//! template inlined.

use ng_transform::{ModuleKind, TransformOptions, transform};

const COMPONENT: &str = r#"import { Component } from '@angular/core';

@Component({
  selector: 'app-foo',
  templateUrl: './foo.component.html',
  styleUrls: ['./foo.component.css'],
})
export class FooComponent {
  title: string = 'hi';
}
"#;

#[test]
fn cjs_lowering_inlines_template_and_strips_types() {
    let opts = TransformOptions {
        module: ModuleKind::CommonJs,
        lower: true,
        ..TransformOptions::default()
    };
    let out = transform(COMPONENT, "foo.component.ts", &opts);
    assert!(out.errors.is_empty(), "errors: {:?}", out.errors);
    eprintln!("--- CJS lowered output ---\n{}", out.code);
    assert!(
        out.code.contains("require(\"./foo.component.html\")"),
        "{}",
        out.code
    );
    // Types stripped.
    assert!(!out.code.contains(": string"), "{}", out.code);
    assert!(!out.code.contains("styleUrls"), "{}", out.code);
}

#[test]
fn esm_lowering_hoists_import() {
    let opts = TransformOptions {
        module: ModuleKind::Esm,
        lower: true,
        ..TransformOptions::default()
    };
    let out = transform(COMPONENT, "foo.component.ts", &opts);
    assert!(out.errors.is_empty(), "errors: {:?}", out.errors);
    eprintln!("--- ESM lowered output ---\n{}", out.code);
    assert!(out.code.contains("__NG_CLI_RESOURCE__0"), "{}", out.code);
}

#[test]
fn target_es2015_downlevels_nullish_coalescing() {
    let opts = TransformOptions {
        target: "es2015".to_string(),
        module: ModuleKind::Esm,
        ..TransformOptions::default()
    };
    let out = transform("export const x = a ?? b;", "x.ts", &opts);
    assert!(out.errors.is_empty(), "errors: {:?}", out.errors);
    // `??` is ES2020; targeting es2015 must lower it away.
    assert!(
        !out.code.contains("??"),
        "should downlevel ??: {}",
        out.code
    );
}

#[test]
fn target_esnext_preserves_modern_syntax() {
    let opts = TransformOptions {
        target: "esnext".to_string(),
        module: ModuleKind::Esm,
        ..TransformOptions::default()
    };
    let out = transform("export const x = a ?? b;", "x.ts", &opts);
    assert!(out.errors.is_empty(), "errors: {:?}", out.errors);
    assert!(out.code.contains("??"), "should preserve ??: {}", out.code);
}

// The "every emitted target round-trips through oxc" canary lives JS-side
// (crates/ng-transform-napi/test/transform.test.mts) where it can iterate the
// real `scriptTargetToString` map over every `ts.ScriptTarget`, tying the JS
// vocabulary to oxc's `EnvOptions::from_target`. This Rust test only pins the
// fail-loud behavior for an unknown string.
#[test]
fn unknown_es_target_is_a_loud_error() {
    // A typo'd/unknown target must surface a diagnostic (the plugins throw on it)
    // rather than silently degrade to no downleveling.
    let opts = TransformOptions {
        target: "es9999".to_string(),
        ..TransformOptions::default()
    };
    let out = transform("export const x = 1;\n", "x.ts", &opts);
    assert!(
        out.errors.iter().any(|e| e.contains("es9999")),
        "expected an unknown-target diagnostic: {:?}",
        out.errors
    );
}
