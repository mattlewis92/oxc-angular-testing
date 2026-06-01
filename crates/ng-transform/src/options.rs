//! Transform options, including the bits we read from the project tsconfig.

/// How a component `templateUrl` is replaced.
///
/// `jest-preset-angular` emits `require(...)` under CommonJS and a top-level
/// `import` under ESM; [`ImportMode::Auto`] reproduces that from the tsconfig
/// `module` setting. The jest plugin defaults to `Auto`, the vitest plugin to
/// [`ImportMode::Import`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImportMode {
    /// Derive from the module kind: CommonJS ⇒ `require`, ESM ⇒ `import`.
    #[default]
    Auto,
    /// Always emit `template: require('./x.html')`.
    Require,
    /// Always emit a hoisted top-level `import __NG_CLI_RESOURCE__N from './x.html'`.
    Import,
}

/// Options controlling the Angular transforms and optional coverage pass.
#[derive(Debug, Clone)]
pub struct TransformOptions {
    /// How `templateUrl` is replaced. See [`ImportMode`].
    pub import_mode: ImportMode,
    /// Whether the resolved module kind is ESM (used when `import_mode` is
    /// [`ImportMode::Auto`]). Derived from tsconfig `module`.
    pub esm: bool,
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
            import_mode: ImportMode::Auto,
            esm: false,
            experimental_decorators: true,
            emit_decorator_metadata: false,
            use_define_for_class_fields: false,
            jit_transforms: true,
            target: "esnext".to_string(),
            lower: true,
            coverage: false,
            coverage_variable: None,
            source_map: true,
        }
    }
}

impl TransformOptions {
    /// Resolve [`ImportMode::Auto`] against the module kind: ESM ⇒ import.
    #[must_use]
    pub fn use_import(&self) -> bool {
        match self.import_mode {
            ImportMode::Auto => self.esm,
            ImportMode::Require => false,
            ImportMode::Import => true,
        }
    }
}
