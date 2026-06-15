//! Transform options, including the bits we read from the project tsconfig.

/// The module format of the emitted code.
///
/// This single setting drives everything that depends on the output format:
/// component `templateUrl` is replaced with `require('./x.html')` under
/// [`ModuleKind::CommonJs`] and a hoisted top-level `import` under
/// [`ModuleKind::Esm`], and the ESM→CommonJS rewrite runs only for `CommonJs`.
/// Derive it from the tsconfig `module` (CommonJS ⇒ `CommonJs`, anything else ⇒
/// `Esm`); the jest plugin does this, and the vitest plugin always uses `Esm`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModuleKind {
    /// `require(...)` / `module.exports`, matching `tsc` `module: "commonjs"`.
    #[default]
    CommonJs,
    /// Top-level `import` / `export`.
    Esm,
}

/// Test runner whose mock-registration calls are hoisted above imports.
///
/// Selects the object name, import module, and hoisted method set for the
/// `babel-plugin-jest-hoist` port. See [`TransformOptions::hoist_mock`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MockFramework {
    /// `jest.*` from the `jest` global or `@jest/globals`. Hoisted methods:
    /// `mock`, `unmock`, `deepUnmock`, `enableAutomock`, `disableAutomock`.
    Jest,
    /// `vi.*` from the `vi` global (vitest `globals: true`) or `vitest`. Hoisted
    /// methods: `mock`, `unmock` (`doMock` / `doUnmock` run in place).
    Vi,
}

/// JSX runtime, mirroring tsconfig `jsx`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JsxRuntime {
    /// `react-jsx` / `react-jsxdev` — auto-imports from `jsxImportSource`
    /// (default `react`). The modern default.
    #[default]
    Automatic,
    /// `react` — classic `React.createElement` (`jsxFactory` / `jsxFragmentFactory`).
    Classic,
}

/// JSX/TSX transform configuration, for repos that mix Angular and React.
///
/// Only affects files that actually contain JSX (`.tsx` / `.jsx`); `.ts` cannot,
/// so this is inert for Angular-only code. Derived from tsconfig `jsx` /
/// `jsxImportSource` / `jsxFactory` / `jsxFragmentFactory`.
#[derive(Debug, Clone, Default)]
pub struct JsxConfig {
    pub runtime: JsxRuntime,
    /// `react-jsxdev`: add `__source` / `__self` debug info.
    pub development: bool,
    /// Automatic runtime import source (default `react` when `None`).
    pub import_source: Option<String>,
    /// Classic runtime factory (default `React.createElement` when `None`).
    pub pragma: Option<String>,
    /// Classic runtime fragment factory (default `React.Fragment` when `None`).
    pub pragma_frag: Option<String>,
}

/// Options controlling the Angular transforms and optional coverage pass.
#[derive(Debug, Clone)]
pub struct TransformOptions {
    /// Output module format. See [`ModuleKind`].
    pub module: ModuleKind,
    /// tsconfig `experimentalDecorators` — enables legacy decorator lowering.
    pub experimental_decorators: bool,
    /// tsconfig `emitDecoratorMetadata`.
    pub emit_decorator_metadata: bool,
    /// tsconfig `useDefineForClassFields`. When `false`, class fields are emitted
    /// as plain assignments (`this.x = …`) rather than `[[Define]]`
    /// (`Object.defineProperty`) semantics — the historical Angular setting that
    /// keeps decorator/DI field initialization working. Maps to oxc's
    /// `set_public_class_fields` + `remove_class_fields_without_initializer`.
    pub use_define_for_class_fields: bool,
    /// Run the Angular compiler-cli JIT transforms (downlevel decorators +
    /// signal initializer-API decorators) before lowering.
    pub jit_transforms: bool,
    /// Hoist mock-registration calls (`mock()` / `unmock()` / …) above imports,
    /// porting `babel-plugin-jest-hoist`. `None` (the default) hoists nothing;
    /// the variant selects the runner (object name, import module, method set).
    ///
    /// The jest plugin sets [`MockFramework::Jest`] for the user's test code:
    /// `jest.mock('./x', factory)` must run before the module under test is
    /// imported, but users write it after their imports.
    ///
    /// Vitest normally relies on its own `vi.mock` hoister and leaves this
    /// `None`. Set [`MockFramework::Vi`] only for code paths that bypass that
    /// hoister (e.g. an in-worker transform that short-circuits the server fetch
    /// where vitest's `hoistMocks` runs), where an un-hoisted `vi.mock` would
    /// register too late and silently no-op.
    pub hoist_mock: Option<MockFramework>,
    /// JSX/TSX transform configuration (mixed Angular + React repos). See
    /// [`JsxConfig`]. Inert for `.ts` (no JSX).
    pub jsx: JsxConfig,
    /// ECMAScript target for syntax downleveling, e.g. `"es2017"`, `"es2022"`,
    /// `"esnext"` (the default). Maps to oxc's `EnvOptions::from_target` — derive
    /// it from tsconfig `target`. Only syntax newer than the target is lowered;
    /// TypeScript stripping and decorator lowering happen regardless.
    pub target: String,
    /// Master switch for the oxc TypeScript → JavaScript + decorator lowering
    /// step. Defaults to `true` (you need JS to run under a test runner). Set
    /// `false` only to inspect the Angular passes' output as TypeScript
    /// (used by the crate's own snapshot tests); the result is not executable.
    pub lower: bool,
    /// Keep component styles instead of stripping them (the default). Style
    /// references are rewritten so the bundler's CSS pipeline compiles them:
    /// `styleUrl`/`styleUrls` entries become default imports (ESM) or
    /// `require(...)` calls (CommonJS) and the property is replaced with
    /// `styles: [...]`; existing inline `styles` are preserved and merged ahead
    /// of the URL-derived entries (Angular's own resolution order). No CSS is
    /// compiled here — that is delegated to the consumer (vite), optionally via
    /// [`keep_styles_query`](Self::keep_styles_query).
    pub keep_styles: bool,
    /// Query parameter appended to each style URL rewritten under
    /// [`keep_styles`](Self::keep_styles): e.g. `"inline"` turns
    /// `./a.scss` into `./a.scss?inline` (`&inline` when the URL already has a
    /// query), which makes vite return the compiled CSS as a string. `None`
    /// (the default) emits the URL verbatim. The vitest plugin passes
    /// `"inline"`.
    pub keep_styles_query: Option<String>,
    /// Instrument the output for istanbul-compatible coverage in the same pass.
    pub coverage: bool,
    /// Global coverage variable name (default `__coverage__`).
    pub coverage_variable: Option<String>,
    /// Emit a source map.
    pub source_map: bool,
}

impl Default for TransformOptions {
    fn default() -> Self {
        Self {
            module: ModuleKind::CommonJs,
            experimental_decorators: true,
            emit_decorator_metadata: false,
            use_define_for_class_fields: false,
            jit_transforms: true,
            hoist_mock: None,
            jsx: JsxConfig::default(),
            target: "esnext".to_string(),
            keep_styles: false,
            keep_styles_query: None,
            lower: true,
            coverage: false,
            coverage_variable: None,
            source_map: true,
        }
    }
}

impl TransformOptions {
    /// Whether the output is ESM (top-level `import` for `templateUrl`, no
    /// ESM→CommonJS rewrite). The inverse selects the CommonJS path.
    #[must_use]
    pub fn is_esm(&self) -> bool {
        matches!(self.module, ModuleKind::Esm)
    }
}
