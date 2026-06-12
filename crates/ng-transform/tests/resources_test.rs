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

fn run_keep_with_query(source: &str, module: ModuleKind, query: Option<&str>) -> String {
    let opts = TransformOptions {
        module,
        keep_styles: true,
        keep_styles_query: query.map(String::from),
        lower: false,
        ..TransformOptions::default()
    };
    let out = transform(source, "app.component.ts", &opts);
    assert!(out.errors.is_empty(), "errors: {:?}", out.errors);
    out.code
}

/// `keep_styles` with the query the vitest plugin uses (`inline`).
fn run_keep(source: &str, module: ModuleKind) -> String {
    run_keep_with_query(source, module, Some("inline"))
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

#[test]
fn template_literal_template_url_is_inlined() {
    // A no-substitution template literal `templateUrl: `./foo.component.html`` is a
    // legal alternative to a string literal; it must inline like the string form
    // rather than being left for a runtime fetch.
    let src = "import { Component } from '@angular/core';\n\
@Component({ templateUrl: `./foo.component.html` })\n\
export class FooComponent {}\n";
    let code = run(src, ModuleKind::CommonJs);
    assert!(
        code.contains("template: require(\"./foo.component.html\")"),
        "template-literal templateUrl not inlined: {code}"
    );
    assert!(!code.contains("templateUrl"), "{code}");
}

// ─── keep_styles ────────────────────────────────────────────────────────────
//
// With `keep_styles`, style URLs are rewritten to `?inline` imports (ESM) or
// requires (CJS) so the bundler's CSS pipeline compiles them; no CSS is
// compiled by the transform itself.

#[test]
fn keep_styles_rewrites_style_urls_to_inline_imports() {
    let src = r#"import { Component } from '@angular/core';
@Component({
  selector: 'app-foo',
  templateUrl: './foo.component.html',
  styleUrls: ['./a.scss', './b.scss'],
  moduleId: 'foo',
})
export class FooComponent {}
"#;
    let code = run_keep(src, ModuleKind::Esm);
    assert!(
        code.contains("import __oxc_ng_style_0__ from \"./a.scss?inline\""),
        "{code}"
    );
    assert!(
        code.contains("import __oxc_ng_style_1__ from \"./b.scss?inline\""),
        "{code}"
    );
    assert!(
        code.contains("styles: [__oxc_ng_style_0__, __oxc_ng_style_1__]"),
        "{code}"
    );
    assert!(!code.contains("styleUrls"), "{code}");
    // templateUrl inlining is unchanged, and moduleId is still stripped.
    assert!(code.contains("template: __NG_CLI_RESOURCE__0"), "{code}");
    assert!(!code.contains("moduleId"), "{code}");
}

#[test]
fn keep_styles_single_entry_style_urls() {
    let src = r#"import { Component } from '@angular/core';
@Component({ styleUrls: ['./a.scss'], template: '' })
export class FooComponent {}
"#;
    let code = run_keep(src, ModuleKind::Esm);
    assert!(
        code.contains("import __oxc_ng_style_0__ from \"./a.scss?inline\""),
        "{code}"
    );
    assert!(code.contains("styles: [__oxc_ng_style_0__]"), "{code}");
    assert!(!code.contains("styleUrls"), "{code}");
}

#[test]
fn keep_styles_singular_style_url() {
    let src = r#"import { Component } from '@angular/core';
@Component({ styleUrl: './a.scss', template: '' })
export class FooComponent {}
"#;
    let code = run_keep(src, ModuleKind::Esm);
    assert!(
        code.contains("import __oxc_ng_style_0__ from \"./a.scss?inline\""),
        "{code}"
    );
    assert!(code.contains("styles: [__oxc_ng_style_0__]"), "{code}");
    assert!(!code.contains("styleUrl:"), "{code}");
}

#[test]
fn keep_styles_inline_styles_only_are_preserved_unchanged() {
    let src = r#"import { Component } from '@angular/core';
@Component({ styles: ['h1 { color: red; }'], template: '' })
export class FooComponent {}
"#;
    let code = run_keep(src, ModuleKind::Esm);
    assert!(code.contains("styles: [\"h1 { color: red; }\"]"), "{code}");
    assert!(!code.contains("?inline"), "{code}");
    assert!(!code.contains("import __oxc_ng_style_"), "{code}");
}

#[test]
fn keep_styles_merges_inline_styles_before_url_styles() {
    // Angular's resolveComponentResources appends fetched styleUrl styles AFTER
    // the inline `styles` array — the merged array must match that order, even
    // though `styleUrl` is declared first here.
    let src = r#"import { Component } from '@angular/core';
@Component({
  styleUrl: './a.scss',
  styles: ['h1 { color: red; }'],
  template: '',
})
export class FooComponent {}
"#;
    let code = run_keep(src, ModuleKind::Esm);
    assert!(
        code.contains("styles: [\"h1 { color: red; }\", __oxc_ng_style_0__]"),
        "{code}"
    );
    assert!(!code.contains("styleUrl:"), "{code}");
}

#[test]
fn keep_styles_merges_string_inline_styles() {
    // Angular accepts `styles: '…'` (a bare string) and normalizes it to a
    // one-element array; the merge must do the same.
    let src = r#"import { Component } from '@angular/core';
@Component({ styles: 'h1 { color: red; }', styleUrl: './a.scss', template: '' })
export class FooComponent {}
"#;
    let code = run_keep(src, ModuleKind::Esm);
    assert!(
        code.contains("styles: [\"h1 { color: red; }\", __oxc_ng_style_0__]"),
        "{code}"
    );
}

#[test]
fn keep_styles_appends_ampersand_inline_to_existing_query() {
    let src = r#"import { Component } from '@angular/core';
@Component({ styleUrl: './a.scss?foo=1', template: '' })
export class FooComponent {}
"#;
    let code = run_keep(src, ModuleKind::Esm);
    assert!(
        code.contains("import __oxc_ng_style_0__ from \"./a.scss?foo=1&inline\""),
        "{code}"
    );
}

#[test]
fn keep_styles_two_components_share_a_stable_counter() {
    let src = r#"import { Component } from '@angular/core';
@Component({ styleUrl: './a.scss', template: '' })
export class AComponent {}
@Component({ styleUrls: ['./b.scss'], template: '' })
export class BComponent {}
"#;
    let code = run_keep(src, ModuleKind::Esm);
    assert!(
        code.contains("import __oxc_ng_style_0__ from \"./a.scss?inline\""),
        "{code}"
    );
    assert!(
        code.contains("import __oxc_ng_style_1__ from \"./b.scss?inline\""),
        "{code}"
    );
    assert!(code.contains("styles: [__oxc_ng_style_0__]"), "{code}");
    assert!(code.contains("styles: [__oxc_ng_style_1__]"), "{code}");
}

#[test]
fn keep_styles_cjs_mode_emits_inline_requires_in_place() {
    let src = r#"import { Component } from '@angular/core';
@Component({ styleUrls: ['./a.scss'], template: '' })
export class FooComponent {}
"#;
    let code = run_keep(src, ModuleKind::CommonJs);
    assert!(
        code.contains("styles: [require(\"./a.scss?inline\")]"),
        "{code}"
    );
    assert!(!code.contains("import __oxc_ng_style_"), "{code}");
}

#[test]
fn keep_styles_normalizes_relative_url_without_dot_prefix() {
    let src = r#"import { Component } from '@angular/core';
@Component({ styleUrl: 'a.scss', template: '' })
export class FooComponent {}
"#;
    let code = run_keep(src, ModuleKind::Esm);
    assert!(code.contains("from \"./a.scss?inline\""), "{code}");
}

#[test]
fn keep_styles_dynamic_style_url_is_left_untouched() {
    // Same policy as a dynamic templateUrl: anything we cannot statically
    // resolve is left alone rather than half-rewritten.
    let src = r#"import { Component } from '@angular/core';
@Component({ styleUrl: SOME_URL, styles: ['h1 {}'], template: '' })
export class FooComponent {}
"#;
    let code = run_keep(src, ModuleKind::Esm);
    assert!(code.contains("styleUrl: SOME_URL"), "{code}");
    assert!(code.contains("styles: [\"h1 {}\"]"), "{code}");
}

#[test]
fn keep_styles_escalates_prefix_on_user_collision() {
    // A user binding spelled like our hoisted identifier prefix must not be
    // shadowed: the prefix deterministically gains a leading underscore.
    let src = r#"import { Component } from '@angular/core';
const __oxc_ng_style_0__ = 'user';
@Component({ styleUrl: './a.scss', template: '' })
export class FooComponent {}
"#;
    let code = run_keep(src, ModuleKind::Esm);
    assert!(
        code.contains("import ___oxc_ng_style_0__ from \"./a.scss?inline\""),
        "{code}"
    );
    assert!(code.contains("styles: [___oxc_ng_style_0__]"), "{code}");
}

#[test]
fn keep_styles_false_still_strips_styles() {
    // Regression: the default (keep_styles: false) keeps the historical
    // jest-preset-angular behavior — all style metadata removed.
    let code = run(COMPONENT, ModuleKind::Esm);
    assert!(!code.contains("styles"), "{code}");
    assert!(!code.contains("styleUrls"), "{code}");
    assert!(!code.contains("?inline"), "{code}");
}

#[test]
fn keep_styles_without_query_emits_url_verbatim() {
    // The default: no query parameter is appended — the rewritten import uses
    // the (normalized) URL exactly as written, existing query included.
    let src = r#"import { Component } from '@angular/core';
@Component({ styleUrls: ['./a.scss', './b.scss?foo=1'], template: '' })
export class FooComponent {}
"#;
    let code = run_keep_with_query(src, ModuleKind::Esm, None);
    assert!(
        code.contains("import __oxc_ng_style_0__ from \"./a.scss\""),
        "{code}"
    );
    assert!(
        code.contains("import __oxc_ng_style_1__ from \"./b.scss?foo=1\""),
        "{code}"
    );
    assert!(!code.contains("inline"), "{code}");
}

#[test]
fn keep_styles_query_is_configurable() {
    let src = r#"import { Component } from '@angular/core';
@Component({ styleUrl: './a.scss', template: '' })
export class FooComponent {}
"#;
    let code = run_keep_with_query(src, ModuleKind::Esm, Some("raw"));
    assert!(
        code.contains("import __oxc_ng_style_0__ from \"./a.scss?raw\""),
        "{code}"
    );
}
