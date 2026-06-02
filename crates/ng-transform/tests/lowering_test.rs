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
