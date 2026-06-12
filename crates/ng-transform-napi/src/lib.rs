//! napi bindings for the Angular test transform.
//!
//! Published as `@oxc-angular-testing/transform` with per-platform binary
//! packages under `@oxc-angular-testing/binding-*`.

#![deny(clippy::all)]

use napi_derive::napi;
use ng_transform::{
    JsxConfig, JsxRuntime, ModuleKind, TransformOptions as NgOptions, transform as ng_transform,
};

/// Options forwarded to the Rust transform. All fields are optional; omitted
/// fields fall back to the Rust-side [`NgOptions`] defaults.
#[napi(object)]
#[derive(Default)]
pub struct TransformOptions {
    /// Output module format: `"commonjs"` (default) or `"esm"`. Drives both the
    /// `templateUrl` replacement (`require` vs top-level `import`) and the
    /// ESM→CommonJS rewrite. Unknown values fall back to `"commonjs"`. Derive it
    /// from tsconfig `module`.
    pub module: Option<String>,
    /// tsconfig `experimentalDecorators`.
    pub experimental_decorators: Option<bool>,
    /// tsconfig `emitDecoratorMetadata`.
    pub emit_decorator_metadata: Option<bool>,
    /// tsconfig `useDefineForClassFields` (default `false` — Angular's setting).
    pub use_define_for_class_fields: Option<bool>,
    /// Run the Angular JIT transforms (downlevel decorators + signal initializer
    /// APIs). Default `true`.
    pub jit_transforms: Option<bool>,
    /// Hoist `jest.mock()` / `jest.unmock()` / etc. above imports
    /// (babel-plugin-jest-hoist). Default `false`; the jest plugin enables it.
    pub hoist_jest_mock: Option<bool>,
    /// JSX runtime for `.tsx`/`.jsx` (mixed Angular + React): `"automatic"`
    /// (default) or `"classic"`. Derive from tsconfig `jsx`.
    pub jsx: Option<String>,
    /// Automatic-runtime import source (default `"react"`). tsconfig `jsxImportSource`.
    pub jsx_import_source: Option<String>,
    /// Classic-runtime factory (default `"React.createElement"`). tsconfig `jsxFactory`.
    pub jsx_factory: Option<String>,
    /// Classic-runtime fragment factory (default `"React.Fragment"`).
    /// tsconfig `jsxFragmentFactory`.
    pub jsx_fragment_factory: Option<String>,
    /// `react-jsxdev`: emit JSX debug info (`__source`/`__self`).
    pub jsx_development: Option<bool>,
    /// ECMAScript target for syntax downleveling (e.g. `"es2017"`, `"esnext"`).
    /// Derive from tsconfig `target`. Default `"esnext"`.
    pub target: Option<String>,
    /// Keep component styles instead of stripping them (default `false`).
    /// `styleUrl`/`styleUrls` become default imports (`"esm"`) or `require(...)`
    /// calls (`"commonjs"`), merged after any inline `styles` — the bundler's
    /// CSS pipeline (e.g. vite + sass) compiles them. No CSS is compiled by the
    /// transform itself. See `keepStylesQuery` for the query parameter that
    /// makes the bundler return the CSS as a string.
    pub keep_styles: Option<bool>,
    /// Query parameter appended to each style URL rewritten under `keepStyles`:
    /// e.g. `"inline"` turns `./a.scss` into `./a.scss?inline` (`&inline` when
    /// the URL already has a query), which makes vite return the compiled CSS
    /// as a string. Default: unset — URLs are emitted verbatim. The vitest
    /// plugin passes `"inline"`.
    pub keep_styles_query: Option<String>,
    /// Master switch for TS→JS + decorator lowering (default `true`; set `false`
    /// only to inspect the pre-lowering TypeScript AST).
    pub lower: Option<bool>,
    /// Instrument the output for istanbul coverage in the same AST pass.
    pub coverage: Option<bool>,
    /// Global coverage variable name (default `"__coverage__"`).
    pub coverage_variable: Option<String>,
    /// Emit a source map (default `true`).
    pub source_map: Option<bool>,
}

/// Result of a transform.
#[napi(object)]
pub struct TransformOutput {
    /// Transformed (and optionally instrumented) JavaScript.
    pub code: String,
    /// Source map JSON, if requested.
    pub map: Option<String>,
    /// Istanbul `FileCoverage` JSON, if `coverage` was set.
    pub coverage_map: Option<String>,
    /// Non-fatal diagnostics.
    pub errors: Vec<String>,
}

fn parse_module(value: Option<&str>) -> ModuleKind {
    match value {
        Some("esm") => ModuleKind::Esm,
        _ => ModuleKind::CommonJs,
    }
}

fn to_ng_options(options: Option<TransformOptions>) -> NgOptions {
    let defaults = NgOptions::default();
    let Some(options) = options else {
        return defaults;
    };
    NgOptions {
        module: parse_module(options.module.as_deref()),
        experimental_decorators: options
            .experimental_decorators
            .unwrap_or(defaults.experimental_decorators),
        emit_decorator_metadata: options
            .emit_decorator_metadata
            .unwrap_or(defaults.emit_decorator_metadata),
        use_define_for_class_fields: options
            .use_define_for_class_fields
            .unwrap_or(defaults.use_define_for_class_fields),
        jit_transforms: options.jit_transforms.unwrap_or(defaults.jit_transforms),
        hoist_jest_mock: options.hoist_jest_mock.unwrap_or(defaults.hoist_jest_mock),
        jsx: JsxConfig {
            runtime: match options.jsx.as_deref() {
                Some("classic") => JsxRuntime::Classic,
                _ => JsxRuntime::Automatic,
            },
            development: options.jsx_development.unwrap_or(false),
            import_source: options.jsx_import_source,
            pragma: options.jsx_factory,
            pragma_frag: options.jsx_fragment_factory,
        },
        target: options.target.unwrap_or(defaults.target),
        keep_styles: options.keep_styles.unwrap_or(defaults.keep_styles),
        keep_styles_query: options.keep_styles_query.or(defaults.keep_styles_query),
        lower: options.lower.unwrap_or(defaults.lower),
        coverage: options.coverage.unwrap_or(defaults.coverage),
        coverage_variable: options.coverage_variable.or(defaults.coverage_variable),
        source_map: options.source_map.unwrap_or(defaults.source_map),
    }
}

/// Transform `source` (the contents of `filename`) for use under a test runner.
#[napi]
pub fn transform(
    source: String,
    filename: String,
    options: Option<TransformOptions>,
) -> TransformOutput {
    let result = ng_transform(&source, &filename, &to_ng_options(options));
    TransformOutput {
        code: result.code,
        map: result.source_map,
        coverage_map: result.coverage_map,
        errors: result.errors,
    }
}
