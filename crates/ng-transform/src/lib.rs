//! Angular test transforms on top of oxc.
//!
//! Reimplements the source transforms `jest-preset-angular` applies to Angular
//! code under test — component resource inlining, Angular decorator
//! downleveling, and the JIT signal-initializer-API decorators — as oxc AST
//! passes, with optional istanbul-compatible coverage instrumentation folded
//! into the *same* parse/codegen via the vendored [`oxc_coverage_instrument`].
//!
//! Pipeline: parse → semantic → \[coverage instrument\] → Angular passes →
//! (oxc TS/decorator lowering) → ESM→CJS → codegen. One
//! [`oxc_allocator::Allocator`], one parse, one codegen.
//!
//! Coverage is instrumented **first**, on the original (TS/JSX) AST, so the
//! istanbul map mirrors the source: it's independent of the ES `target` (no
//! `?.`/`??`/`async` branch reshaping) and never counts compiler-synthesized
//! nodes (the field-init constructor, `ctorParameters` arrows, the
//! dynamic-import wrapper). The inserted counters ride through the transforms;
//! the preamble is prepended at the single codegen.

mod esm_to_cjs;
mod jest_hoist;
mod jit_transform;
mod options;
mod resources;
mod runtime_helpers;

pub use options::{JsxConfig, JsxRuntime, ModuleKind, TransformOptions};

use std::path::Path;

use oxc_allocator::Allocator;
use oxc_codegen::{Codegen, CodegenOptions};
use oxc_coverage_instrument::{InstrumentOptions, instrument_program_ast};
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_transformer::{
    CompilerAssumptions, DecoratorOptions, EnvOptions, JsxOptions, JsxRuntime as OxcJsxRuntime,
    Module, TransformOptions as OxcTransformOptions, Transformer, TypeScriptOptions,
};
use oxc_traverse::traverse_mut;

use jest_hoist::JestHoist;
use jit_transform::JitTransform;
use resources::ResourceTransform;

/// Result of a [`transform`] call.
#[derive(Debug, Clone)]
pub struct TransformResult {
    /// The transformed (and optionally instrumented) JavaScript.
    pub code: String,
    /// Source map JSON, when [`TransformOptions::source_map`] is set.
    pub source_map: Option<String>,
    /// Istanbul `FileCoverage` JSON, when [`TransformOptions::coverage`] is set.
    pub coverage_map: Option<String>,
    /// Non-fatal diagnostics (parse/transform errors rendered as strings).
    pub errors: Vec<String>,
}

/// Transform `source` (the contents of `filename`) for use under a test runner.
///
/// `filename` drives the [`SourceType`] (ts/tsx/js/jsx) and is used as the
/// source-map / coverage path.
#[must_use]
pub fn transform(source: &str, filename: &str, options: &TransformOptions) -> TransformResult {
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(filename).unwrap_or_default();
    let parsed = Parser::new(&allocator, source, source_type).parse();
    let mut errors: Vec<String> = parsed.errors.iter().map(ToString::to_string).collect();
    let mut program = parsed.program;
    // Set when the ESM→CJS pass runs; `cjs_prelude` holds the interop helpers to
    // emit after `"use strict";`.
    let mut did_cjs = false;
    let mut cjs_prelude = String::new();
    // Inlined async helper(s) (`var _asyncToGenerator = …`) when async is
    // downleveled — see `runtime_helpers`.
    let mut async_helper_prelude = String::new();

    // Coverage is instrumented up front, on the original AST, before any
    // transform reshapes or synthesizes nodes — so the map reflects the source.
    // The counters ride through the transforms below; `coverage_preamble` (the
    // `var __coverage__…` IIFE) is prepended at codegen.
    let mut coverage_map: Option<String> = None;
    let mut coverage_preamble = String::new();
    if options.coverage {
        let cov_opts = InstrumentOptions {
            coverage_variable: options
                .coverage_variable
                .clone()
                .unwrap_or_else(|| "__coverage__".to_string()),
            source_map: options.source_map,
            ..InstrumentOptions::default()
        };
        match instrument_program_ast(&allocator, &mut program, source, filename, &cov_opts) {
            Ok(result) => {
                coverage_map = Some(result.coverage_map_json);
                coverage_preamble = result.preamble;
            }
            Err(err) => errors.push(err.to_string()),
        }
    }

    // Angular passes mutate `program` in place on `allocator`:
    //   resources (templateUrl/styles) → JIT (signal initializer APIs + downlevel
    //   decorators). Each rebuilds scoping since it inserts/removes nodes.
    {
        let scoping = SemanticBuilder::new()
            .build(&program)
            .semantic
            .into_scoping();
        let mut resources = ResourceTransform::new(options.is_esm());
        traverse_mut(&mut resources, &allocator, &mut program, scoping, ());
    }
    if options.jit_transforms {
        let scoping = SemanticBuilder::new()
            .build(&program)
            .semantic
            .into_scoping();
        let mut jit = JitTransform::new();
        traverse_mut(&mut jit, &allocator, &mut program, scoping, ());
    }

    // Hoist `jest.mock()` above imports (babel-plugin-jest-hoist), before the
    // ESM→CJS rewrite so the hoisted call lands above the generated requires.
    if options.hoist_jest_mock {
        let scoping = SemanticBuilder::new()
            .build(&program)
            .semantic
            .into_scoping();
        let mut hoist = JestHoist::new();
        traverse_mut(&mut hoist, &allocator, &mut program, scoping, ());
    }

    // TypeScript → JavaScript + legacy-decorator lowering, so the output is
    // executable under the test runner. `Module::CommonJS` is selected for the
    // require path (jest/CJS), ESM otherwise (vitest).
    if options.lower {
        let scoping = SemanticBuilder::new()
            .build(&program)
            .semantic
            .into_scoping();
        // ESM emits import/export; for CJS we keep the modules untouched here and
        // do the ESM→CJS rewrite (incl. `"use strict"`) ourselves below.
        let module = if options.is_esm() {
            Module::Esm
        } else {
            Module::Preserve
        };
        // tsconfig `useDefineForClassFields: false` ⇒ emit class fields as plain
        // assignments (oxc: `set_public_class_fields` + strip uninitialized fields).
        let use_define = options.use_define_for_class_fields;
        // ES target drives syntax downleveling; fall back to no downleveling on a
        // bad target string. Then layer in the module format.
        let mut env = EnvOptions::from_target(&options.target).unwrap_or_default();
        env.module = module;
        // `async`/`await` downlevels per `target`; the helper it pulls in is then
        // inlined (see `inline_async_helpers` below) so the result Promise is the
        // module's global (zone-patched), matching tsc.
        // JSX/TSX (mixed Angular + React). Enabled unconditionally — `.ts` has no
        // JSX so this is inert there; only `.tsx`/`.jsx` are transformed. Runtime
        // + source/factory come from the tsconfig-derived `jsx` config.
        let mut jsx = JsxOptions::enable();
        jsx.runtime = match options.jsx.runtime {
            JsxRuntime::Automatic => OxcJsxRuntime::Automatic,
            JsxRuntime::Classic => OxcJsxRuntime::Classic,
        };
        jsx.development = options.jsx.development;
        jsx.import_source = options.jsx.import_source.clone();
        jsx.pragma = options.jsx.pragma.clone();
        jsx.pragma_frag = options.jsx.pragma_frag.clone();
        jsx.conform(); // dev mode needs the self/source plugins on
        let oxc_options = OxcTransformOptions {
            typescript: TypeScriptOptions {
                remove_class_fields_without_initializer: !use_define,
                ..TypeScriptOptions::default()
            },
            assumptions: CompilerAssumptions {
                set_public_class_fields: !use_define,
                ..CompilerAssumptions::default()
            },
            jsx,
            decorator: DecoratorOptions {
                legacy: options.experimental_decorators,
                emit_decorator_metadata: options.emit_decorator_metadata,
            },
            env,
            // Runtime mode imports decorator/class helpers from `@oxc-project/runtime`
            // (a dependency of `@oxc-angular-testing/transform`). Inline mode is not
            // yet implemented in oxc 0.126.
            ..OxcTransformOptions::default()
        };
        let ret = Transformer::new(&allocator, Path::new(filename), &oxc_options)
            .build_with_scoping(scoping, &mut program);
        errors.extend(ret.errors.iter().map(ToString::to_string));

        // Inline the async runtime helper (drop its import) so a downleveled
        // `async` returns the module's global (zone-patched) Promise. Runs for
        // both CJS and ESM output, before the ESM→CJS rewrite below.
        async_helper_prelude = runtime_helpers::inline_async_helpers(&allocator, &mut program);

        // CJS mode: rewrite ESM import/export to CommonJS, matching TypeScript's
        // `esModuleInterop` emit. Returns the interop helper prelude text.
        if !options.is_esm() {
            cjs_prelude = esm_to_cjs::esm_to_cjs(&allocator, &mut program);
            did_cjs = true;
        }
    }

    let codegen_options = CodegenOptions {
        source_map_path: options
            .source_map
            .then(|| std::path::PathBuf::from(filename)),
        ..CodegenOptions::default()
    };
    let ret = Codegen::new()
        .with_options(codegen_options)
        .with_source_text(source)
        .build(&program);

    let (code, prefix_lines) = assemble(
        did_cjs,
        &cjs_prelude,
        &async_helper_prelude,
        &coverage_preamble,
        ret.code,
    );
    TransformResult {
        code,
        source_map: ret
            .map
            .map(|map| offset_source_map(&map.to_json_string(), prefix_lines)),
        coverage_map,
        errors,
    }
}

/// Prepend, in order: `"use strict";` + the CJS interop prelude (CJS only), the
/// inlined async helper(s), then the coverage preamble (when instrumenting).
/// Returns the assembled code and the number of prepended lines, so the source
/// map can be shifted to match (see [`offset_source_map`]).
fn assemble(
    did_cjs: bool,
    cjs_prelude: &str,
    async_helper_prelude: &str,
    coverage_preamble: &str,
    code: String,
) -> (String, usize) {
    let mut prefix = String::new();
    if did_cjs {
        prefix.push_str("\"use strict\";\n");
        prefix.push_str(cjs_prelude);
    }
    prefix.push_str(async_helper_prelude);
    if !coverage_preamble.is_empty() {
        prefix.push_str(coverage_preamble);
        if !coverage_preamble.ends_with('\n') {
            prefix.push('\n');
        }
    }
    if prefix.is_empty() {
        return (code, 0);
    }
    let prefix_lines = prefix.bytes().filter(|&b| b == b'\n').count();
    (format!("{prefix}{code}"), prefix_lines)
}

/// Shift every mapping in a source map down by `lines` generated lines.
///
/// We prepend `"use strict";` + interop helpers as raw text after codegen, so
/// the generated code starts `lines` rows lower than the map (built against the
/// codegen output) assumes. A VLQ `mappings` string encodes generated lines as
/// `;`-separated groups, so prepending `lines` semicolons inserts that many
/// empty leading lines — without disturbing the (file-relative) source/name
/// deltas, which only advance on real segments. No-op when `lines == 0`.
fn offset_source_map(map_json: &str, lines: usize) -> String {
    if lines == 0 {
        return map_json.to_string();
    }
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(map_json) else {
        return map_json.to_string();
    };
    if let Some(mappings) = value.get_mut("mappings").and_then(|m| m.as_str()) {
        let shifted = format!("{};{mappings}", ";".repeat(lines - 1));
        value["mappings"] = serde_json::Value::String(shifted);
    }
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_prepends_semicolons_without_touching_source_deltas() {
        let map = r#"{"version":3,"sources":["m.ts"],"names":[],"mappings":"AAAA,CAAC;AACD"}"#;
        let out = offset_source_map(map, 4);
        let value: serde_json::Value = serde_json::from_str(&out).unwrap();
        let mappings = value["mappings"].as_str().unwrap();
        // 4 leading empty lines, then the original mappings unchanged.
        assert_eq!(mappings, ";;;;AAAA,CAAC;AACD");
    }

    #[test]
    fn offset_zero_is_a_no_op() {
        let map = r#"{"version":3,"sources":["m.ts"],"names":[],"mappings":"AAAA"}"#;
        assert_eq!(offset_source_map(map, 0), map);
    }

    #[test]
    fn cjs_source_map_is_shifted_by_the_prelude() {
        // A default import emits `"use strict";` + the `__importDefault` helper
        // ahead of the generated code, so the map must gain that many empty
        // leading lines — otherwise every position is reported too high.
        let opts = TransformOptions {
            module: ModuleKind::CommonJs,
            target: "es2022".to_string(),
            jit_transforms: false,
            source_map: true,
            ..TransformOptions::default()
        };
        let out = transform("import d from './m';\nd();\n", "m.ts", &opts);
        let map = out.source_map.expect("source map");
        let value: serde_json::Value = serde_json::from_str(&map).unwrap();
        let mappings = value["mappings"].as_str().unwrap();
        let lead = mappings.bytes().take_while(|&b| b == b';').count();
        // The generated code starts this many lines down (`"use strict";` + the
        // multi-line `__importDefault` helper, before the `__esModule` marker).
        let prelude_lines = out
            .code
            .lines()
            .take_while(|l| !l.contains("__esModule"))
            .count();
        assert!(prelude_lines >= 2, "sanity: prelude present");
        // Without the offset, `lead` would only cover codegen's own unmapped
        // header (< prelude_lines); the fix shifts the whole map down past the
        // prepended prelude.
        assert!(
            lead >= prelude_lines,
            "map not shifted by the prelude: {lead} leading lines < prelude height {prelude_lines}: {mappings}"
        );
    }
}
