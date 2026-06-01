//! napi bindings for the Angular test transform.
//!
//! Published as `@oxc-angular-testing/transform` with per-platform binary
//! packages under `@oxc-angular-testing/binding-*`.

#![deny(clippy::all)]

use napi_derive::napi;
use ng_transform::{ImportMode, TransformOptions as NgOptions, transform as ng_transform};

/// Options forwarded to the Rust transform. All fields are optional; omitted
/// fields fall back to the Rust-side [`NgOptions`] defaults.
#[napi(object)]
#[derive(Default)]
pub struct TransformOptions {
    /// `"auto"` (default), `"require"`, or `"import"`. Controls how `templateUrl`
    /// is replaced. Unknown values fall back to `"auto"`.
    pub import_mode: Option<String>,
    /// Whether the resolved module kind is ESM (used when `importMode` is
    /// `"auto"`). Derived by the caller from tsconfig `module`.
    pub esm: Option<bool>,
    /// tsconfig `experimentalDecorators`.
    pub experimental_decorators: Option<bool>,
    /// tsconfig `emitDecoratorMetadata`.
    pub emit_decorator_metadata: Option<bool>,
    /// tsconfig `useDefineForClassFields` (default `false` — Angular's setting).
    pub use_define_for_class_fields: Option<bool>,
    /// Run the Angular JIT transforms (downlevel decorators + signal initializer
    /// APIs). Default `true`.
    pub jit_transforms: Option<bool>,
    /// ECMAScript target for syntax downleveling (e.g. `"es2017"`, `"esnext"`).
    /// Derive from tsconfig `target`. Default `"esnext"`.
    pub target: Option<String>,
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

fn parse_import_mode(value: Option<&str>) -> ImportMode {
    match value {
        Some("require") => ImportMode::Require,
        Some("import") => ImportMode::Import,
        _ => ImportMode::Auto,
    }
}

fn to_ng_options(options: Option<TransformOptions>) -> NgOptions {
    let defaults = NgOptions::default();
    let Some(options) = options else {
        return defaults;
    };
    NgOptions {
        import_mode: parse_import_mode(options.import_mode.as_deref()),
        esm: options.esm.unwrap_or(defaults.esm),
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
        target: options.target.unwrap_or(defaults.target),
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
