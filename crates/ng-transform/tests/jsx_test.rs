//! JSX/TSX transform (for repos mixing Angular and React), driven by the
//! tsconfig-derived `jsx` config.

use ng_transform::{JsxConfig, JsxRuntime, ModuleKind, TransformOptions, transform};

const TSX: &str = "export const App = () => <div className=\"a\">hi {name}</div>;\n";

fn run(jsx: JsxConfig, module: ModuleKind) -> String {
    let opts = TransformOptions {
        module,
        jsx,
        jit_transforms: false,
        ..TransformOptions::default()
    };
    let out = transform(TSX, "App.tsx", &opts);
    assert!(out.errors.is_empty(), "errors: {:?}", out.errors);
    out.code
}

#[test]
fn automatic_runtime_uses_jsx_runtime_import() {
    let code = run(JsxConfig::default(), ModuleKind::Esm);
    assert!(code.contains("react/jsx-runtime"), "{code}");
    // `_jsx("div", …)` / `_jsxs("div", …)` (multi-child uses the `s` variant).
    assert!(code.contains("(\"div\", {"), "{code}");
    assert!(!code.contains("React.createElement"), "{code}");
}

#[test]
fn automatic_runtime_honors_custom_import_source() {
    let jsx = JsxConfig {
        runtime: JsxRuntime::Automatic,
        import_source: Some("@emotion/react".to_string()),
        ..JsxConfig::default()
    };
    let code = run(jsx, ModuleKind::Esm);
    assert!(code.contains("@emotion/react/jsx-runtime"), "{code}");
}

#[test]
fn classic_runtime_uses_react_create_element() {
    let jsx = JsxConfig {
        runtime: JsxRuntime::Classic,
        ..JsxConfig::default()
    };
    let code = run(jsx, ModuleKind::Esm);
    assert!(code.contains("React.createElement(\"div\""), "{code}");
    assert!(!code.contains("jsx-runtime"), "{code}");
}

#[test]
fn classic_runtime_honors_custom_pragma() {
    let jsx = JsxConfig {
        runtime: JsxRuntime::Classic,
        pragma: Some("h".to_string()),
        pragma_frag: Some("Fragment".to_string()),
        ..JsxConfig::default()
    };
    let code = run(jsx, ModuleKind::Esm);
    assert!(code.contains("h(\"div\""), "{code}");
    assert!(!code.contains("React.createElement"), "{code}");
}

#[test]
fn cjs_output_requires_the_jsx_runtime() {
    // Automatic runtime under CommonJS: the auto-injected jsx-runtime import is
    // rewritten to a require by the ESM→CJS pass.
    let code = run(JsxConfig::default(), ModuleKind::CommonJs);
    assert!(code.contains(r#"require("react/jsx-runtime")"#), "{code}");
}

#[test]
fn plain_ts_is_unaffected_by_jsx_being_enabled() {
    // `.ts` cannot contain JSX, so enabling the JSX plugin is inert there.
    let opts = TransformOptions {
        module: ModuleKind::CommonJs,
        jit_transforms: false,
        ..TransformOptions::default()
    };
    let out = transform(
        "export const lt = a < b;\nexport const x = 1;\n",
        "m.ts",
        &opts,
    );
    assert!(out.errors.is_empty(), "errors: {:?}", out.errors);
    assert!(out.code.contains("a < b"), "{}", out.code);
    assert!(!out.code.contains("jsx-runtime"), "{}", out.code);
}
