//! Ported jest-preset-angular `replaceResources` cases.

use ng_transform::{ModuleKind, TransformOptions, transform};

fn run(source: &str, module: ModuleKind) -> String {
    // Disable TS/decorator lowering so we snapshot the resource pass in isolation.
    let opts = TransformOptions {
        module,
        lower: false,
        ..TransformOptions::default()
    };
    let out = transform(source, "app.component.ts", &opts);
    assert!(out.errors.is_empty(), "errors: {:?}", out.errors);
    out.code
}

const COMPONENT: &str = r#"import { Component } from '@angular/core';

@Component({
  selector: 'app-foo',
  templateUrl: './foo.component.html',
  styleUrls: ['./foo.component.css'],
  styles: ['h1 { color: red; }'],
  moduleId: 'foo',
})
export class FooComponent {}
"#;

#[test]
fn require_mode_inlines_template_and_strips_styles() {
    let code = run(COMPONENT, ModuleKind::CommonJs);
    assert!(
        code.contains("template: require(\"./foo.component.html\")"),
        "{code}"
    );
    assert!(!code.contains("templateUrl"), "{code}");
    assert!(!code.contains("styleUrls"), "{code}");
    assert!(!code.contains("styles"), "{code}");
    assert!(!code.contains("moduleId"), "{code}");
    assert!(
        !code.contains("require(\"./foo.component.css\")"),
        "styles must not be required: {code}"
    );
}

#[test]
fn esm_module_hoists_top_level_import() {
    let code = run(COMPONENT, ModuleKind::Esm);
    assert!(
        code.contains("import __NG_CLI_RESOURCE__0 from \"./foo.component.html\""),
        "{code}"
    );
    assert!(code.contains("template: __NG_CLI_RESOURCE__0"), "{code}");
    assert!(!code.contains("templateUrl"), "{code}");
    assert!(!code.contains("require("), "{code}");
}

#[test]
fn normalizes_relative_path_without_dot_prefix() {
    let src = r#"import { Component } from '@angular/core';
@Component({ templateUrl: 'foo.html' })
export class FooComponent {}
"#;
    let code = run(src, ModuleKind::CommonJs);
    assert!(code.contains("require(\"./foo.html\")"), "{code}");
}

#[test]
fn ignores_components_not_from_angular_core() {
    let src = r#"import { Component } from 'not-angular';
@Component({ templateUrl: './foo.html' })
export class FooComponent {}
"#;
    let code = run(src, ModuleKind::CommonJs);
    // No @angular/core import → leave untouched.
    assert!(code.contains("templateUrl: \"./foo.html\""), "{code}");
}

#[test]
fn handles_aliased_component_import() {
    let src = r#"import { Component as NgComponent } from '@angular/core';
@NgComponent({ templateUrl: './foo.html' })
export class FooComponent {}
"#;
    let code = run(src, ModuleKind::CommonJs);
    assert!(code.contains("template: require(\"./foo.html\")"), "{code}");
}

#[test]
fn handles_single_style_url() {
    let src = r#"import { Component } from '@angular/core';
@Component({ templateUrl: './foo.html', styleUrl: './foo.css' })
export class FooComponent {}
"#;
    let code = run(src, ModuleKind::CommonJs);
    assert!(!code.contains("styleUrl"), "{code}");
    assert!(code.contains("template: require(\"./foo.html\")"), "{code}");
}

#[test]
fn namespace_component_inlines_template_and_strips_styles() {
    // `import * as ng` + `@ng.Component({ templateUrl })`: resources.rs must inline
    // the templateUrl for the namespace form too (parity with jit_transform), not
    // leave it for a runtime fetch.
    let src = r#"import * as ng from '@angular/core';
@ng.Component({
  selector: 'app-foo',
  templateUrl: './foo.component.html',
  styleUrls: ['./foo.component.css'],
})
export class FooComponent {}
"#;
    let code = run(src, ModuleKind::CommonJs);
    assert!(
        code.contains("template: require(\"./foo.component.html\")"),
        "namespace @ng.Component templateUrl not inlined: {code}"
    );
    assert!(!code.contains("templateUrl"), "{code}");
    assert!(!code.contains("styleUrls"), "{code}");
}
