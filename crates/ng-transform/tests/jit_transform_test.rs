//! Angular compiler-cli JIT transforms (downlevel decorators + signal
//! initializer-API decorators), ported from `@angular/compiler-cli`.
//! These snapshot the JIT passes pre-lowering (`lower: false`).

use ng_transform::{TransformOptions, transform};

fn ts(source: &str) -> String {
    let opts = TransformOptions {
        lower: false,
        ..TransformOptions::default()
    };
    let out = transform(source, "app.ts", &opts);
    assert!(out.errors.is_empty(), "errors: {:?}", out.errors);
    out.code
}

#[test]
fn injectable_ctor_parameters_are_downleveled() {
    let code = ts(r#"import { Injectable } from '@angular/core';
export class Dep {}
@Injectable()
export class MyService {
  constructor(dep: Dep) {}
}
"#);
    assert!(
        code.contains("static ctorParameters = () => [{ type: Dep }]"),
        "{code}"
    );
}

#[test]
fn ctor_param_decorators_are_captured_and_stripped() {
    let code = ts(
        r#"import { Directive, Inject, Optional } from '@angular/core';
class Svc {}
@Directive()
export class MyDir {
  constructor(@Inject('TOK') token: any, @Optional() svc: Svc) {}
}
"#,
    );
    // Param decorators removed from the signature.
    assert!(
        code.contains("constructor(token, svc)")
            || code.contains("constructor(token: any, svc: Svc)"),
        "{code}"
    );
    assert!(code.contains(r#"type: Inject"#), "{code}");
    assert!(code.contains(r#"args: ["TOK"]"#), "{code}");
    assert!(code.contains(r#"type: Optional"#), "{code}");
    // `any` → undefined; class type → the value reference.
    assert!(code.contains("type: undefined"), "{code}");
    assert!(code.contains("type: Svc"), "{code}");
}

#[test]
fn member_decorators_become_prop_decorators() {
    let code = ts(r#"import { Directive, Input, Output } from '@angular/core';
@Directive()
export class MyDir {
  @Input() disabled = false;
  @Output() change = null;
}
"#);
    assert!(code.contains("static propDecorators ="), "{code}");
    assert!(code.contains("disabled: [{ type: Input }]"), "{code}");
    assert!(code.contains("change: [{ type: Output }]"), "{code}");
    // Original member decorators stripped.
    assert!(!code.contains("@Input()"), "{code}");
    assert!(!code.contains("@Output()"), "{code}");
}

#[test]
fn non_angular_member_decorators_are_left_in_place() {
    let code = ts(r#"import { Directive, Input } from '@angular/core';
import { Custom } from './custom';
@Directive()
export class MyDir {
  @Input() @Custom() disabled = false;
}
"#);
    // Custom decorator stays on the member; only @Input is downleveled.
    assert!(code.contains("@Custom()"), "{code}");
    assert!(code.contains("disabled: [{ type: Input }]"), "{code}");
}

#[test]
fn signal_input_and_output_gain_decorators_via_prop_decorators() {
    let code = ts(r#"import { Component, input, output } from '@angular/core';
@Component({ template: '' })
export class MyComponent {
  disabled = input<boolean>(false);
  required = input.required<string>();
  changed = output<string>();
}
"#);
    // input() → @Input({ isSignal, alias, required, transform }) → propDecorators.
    assert!(code.contains("static propDecorators ="), "{code}");
    assert!(code.contains("isSignal: true"), "{code}");
    assert!(code.contains(r#"alias: "disabled""#), "{code}");
    assert!(code.contains("required: true"), "{code}"); // for input.required
    assert!(code.contains("transform: undefined"), "{code}");
    // output() → @Output("changed").
    assert!(code.contains(r#"type: Output"#), "{code}");
    assert!(code.contains(r#"args: ["changed"]"#), "{code}");
    // Input/Output auto-imported from @angular/core.
    assert!(code.contains("Input") && code.contains("Output"), "{code}");
}

#[test]
fn signal_model_emits_input_and_change_output() {
    let code = ts(r#"import { Component, model } from '@angular/core';
@Component({ template: '' })
export class MyComponent {
  value = model<number>(0);
}
"#);
    assert!(code.contains("isSignal: true"), "{code}");
    assert!(code.contains(r#"alias: "value""#), "{code}");
    assert!(code.contains(r#"args: ["valueChange"]"#), "{code}");
}

#[test]
fn signal_view_child_query_is_registered() {
    let code = ts(r#"import { Component, viewChild } from '@angular/core';
@Component({ template: '' })
export class MyComponent {
  ref = viewChild<unknown>('tpl');
}
"#);
    assert!(code.contains("ref: [{"), "{code}");
    assert!(code.contains("type: ViewChild"), "{code}");
    assert!(code.contains("isSignal: true"), "{code}");
}

#[test]
fn ignores_classes_without_angular_decorators() {
    let code = ts(r#"export class Plain {
  constructor(x: number) {}
}
"#);
    assert!(!code.contains("ctorParameters"), "{code}");
    assert!(!code.contains("propDecorators"), "{code}");
}
