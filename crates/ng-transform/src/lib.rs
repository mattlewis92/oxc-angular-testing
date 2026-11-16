//! Angular test transforms on top of oxc.
//!
//! Reimplements the source transforms `jest-preset-angular` applies to Angular
//! code under test — component resource inlining, Angular decorator
//! downleveling, and the JIT signal-initializer-API decorators — as oxc AST
//! passes, with optional istanbul-compatible coverage instrumentation folded
//! into the *same* parse/codegen via [`oxc_coverage_instrument`]'s AST-level
//! `instrument_program` entry (its `ast-api` feature).
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

mod delegate_ctor;
mod enum_ref_fold;
mod esm_to_cjs;
mod jit_transform;
mod mock_hoist;
mod options;
mod resources;
mod strict_null_strip;

pub use options::{JsxConfig, JsxRuntime, MockFramework, ModuleKind, TransformOptions};

use std::path::Path;

use oxc_allocator::Allocator;
use oxc_codegen::{Codegen, CodegenOptions};
use oxc_coverage_instrument::{InstrumentOptions, instrument_program};
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_transformer::{
    CompilerAssumptions, DecoratorOptions, EnvOptions, HelperLoaderOptions, JsxOptions,
    JsxRuntime as OxcJsxRuntime, Module, TransformOptions as OxcTransformOptions, Transformer,
    TypeScriptOptions,
};
use oxc_traverse::traverse_mut;

use delegate_ctor::DelegateCtorTransform;
use jit_transform::JitTransform;
use mock_hoist::MockHoist;
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
    /// Diagnostics (parse/transform errors, an unknown ES target, …) rendered as
    /// strings. **Callers MUST inspect this.** `transform` never hard-fails — it
    /// always returns whatever `code` it produced, even when `errors` is non-empty
    /// (the output may then be incomplete or wrong). A caller that ignores `errors`
    /// can silently ship miscompiled code. Both bundled plugins fail the run when it
    /// is non-empty (jest throws, vitest calls `this.error`); any other caller must
    /// do the same.
    pub errors: Vec<String>,
}

/// Transform `source` (the contents of `filename`) for use under a test runner.
///
/// `filename` drives the [`SourceType`] (ts/tsx/js/jsx) and is used as the
/// source-map / coverage path.
///
/// # Errors
///
/// Errors are not returned via `Result` — they are accumulated into
/// [`TransformResult::errors`], which the caller **must** inspect (see that field).
/// `transform` always returns a [`TransformResult`]; a non-empty `errors` means the
/// returned `code` may be incomplete or incorrect.
#[must_use]
pub fn transform(source: &str, filename: &str, options: &TransformOptions) -> TransformResult {
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(filename).unwrap_or_default();
    let parsed = Parser::new(&allocator, source, source_type).parse();
    let mut errors: Vec<String> = parsed
        .diagnostics
        .errors()
        .map(ToString::to_string)
        .collect();
    let mut program = parsed.program;
    // Set when the ESM→CJS pass runs; `cjs_prelude` holds the interop helpers to
    // emit after `"use strict";`.
    let mut did_cjs = false;
    let mut cjs_prelude = String::new();

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
        let scoping = SemanticBuilder::new()
            .build(&program)
            .semantic
            .into_scoping();
        // Unhandled pragmas (`result.unhandled_pragmas`) are dropped: the host has
        // no warnings channel. Scoping is rebuilt by every pass below, so the
        // returned (pre-preamble) `result.scoping` isn't needed either.
        match instrument_program(
            &allocator,
            &mut program,
            scoping,
            source,
            filename,
            &cov_opts,
        ) {
            Ok(result) => {
                coverage_preamble = result
                    .preamble
                    .map(|p| with_coverage_schema(&p))
                    .unwrap_or_default();
                coverage_map = Some(result.coverage_map_json);
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
        let mut resources = ResourceTransform::new(
            options.is_esm(),
            options.keep_styles,
            options.keep_styles_query.clone(),
            source,
        );
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

    // Hoist `jest.mock()` / `vi.mock()` above imports (babel-plugin-jest-hoist),
    // before the ESM→CJS rewrite so the hoisted call lands above the generated
    // requires.
    if let Some(framework) = options.hoist_mock {
        let scoping = SemanticBuilder::new()
            .build(&program)
            .semantic
            .into_scoping();
        let mut hoist = MockHoist::new(framework);
        traverse_mut(&mut hoist, &allocator, &mut program, scoping, ());
    }

    // tsconfig `strictNullChecks: false` metadata parity: strip `null`/
    // `undefined` from union type annotations before the serializer sees them
    // (see `strict_null_strip`). Type-only — needs no scoping. oxc 0.157's own
    // `DecoratorOptions::strict_null_checks` (also passed below) is NOT a
    // substitute: it still serializes a `Date | null` ctor param as `Object`
    // (pinned by the napi `strictNullChecks=false` test).
    if options.lower && options.emit_decorator_metadata && !options.strict_null_checks {
        use oxc_ast_visit::VisitMut;
        strict_null_strip::StripNullableUnions.visit_program(&mut program);
    }

    // Fold same-file enum-member references in enum initializers to their
    // literals (tsc parity — see `enum_ref_fold`), so oxc's enum lowering emits
    // the string-enum form instead of a bogus reverse mapping.
    if options.lower {
        enum_ref_fold::fold_same_file_enum_refs(
            &mut program,
            &oxc_ast::builder::AstBuilder::new(&allocator),
        );
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
        // ES target drives syntax downleveling. An unrecognized target must NOT be
        // silently swallowed into the default (no-downleveling) env: for this project
        // that is a silent miscompile (async stops being downleveled, so it is no
        // longer zone-aware). Push a diagnostic — the plugins throw on `errors` — so
        // a typo'd target or an oxc rename fails loudly instead. Then layer in module.
        let mut env = match EnvOptions::from_target(&options.target) {
            Ok(env) => env,
            Err(err) => {
                errors.push(format!(
                    "unknown ES target {:?}: {err} (expected es5–es2024 or esnext)",
                    options.target
                ));
                EnvOptions::default()
            }
        };
        env.module = module;
        // BigInt literals cannot be downleveled; oxc's `big_int` flag exists only
        // to hard-error on them when the target predates ES2020. tsc in
        // transpile-only mode (the ts-jest/isolatedModules baseline this transform
        // replaces) passes them through untouched — TS2737 is a checker
        // diagnostic, and nothing type-checks here. Match tsc: never error.
        env.es2020.big_int = false;
        // `async`/`await` downlevels per `target`, pulling in oxc's runtime
        // `asyncToGenerator` helper (imported from `@oxc-project/runtime`). Its
        // bare, late-bound `new Promise` resolves to the realm-global `Promise` at
        // call time, so under zone.js the result is the zone-patched Promise —
        // matching tsc/ts-jest. (The native, non-downleveled async path — esnext
        // target — cannot be made zone-aware: it uses the V8 %Promise% intrinsic
        // zone.js never replaces. That is why specs target es2016.)
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
                strict_null_checks: options.strict_null_checks,
            },
            helper_loader: HelperLoaderOptions {
                module_name: options
                    .helper_module_name
                    .clone()
                    .map_or_else(|| HelperLoaderOptions::default().module_name, Into::into),
                ..HelperLoaderOptions::default()
            },
            env,
            // Runtime mode imports decorator/class helpers from `@oxc-project/runtime`
            // (a dependency of `@oxc-angular-testing/transform`). Inline mode is not
            // yet implemented in oxc 0.157.
            ..OxcTransformOptions::default()
        };
        let ret = Transformer::new(&allocator, Path::new(filename), &oxc_options)
            .build_with_scoping(scoping, &mut program);
        errors.extend(ret.diagnostics.errors().map(ToString::to_string));

        // oxc synthesizes a derived class's field-init constructor as
        // `constructor(..._args) { super(..._args); /*fields*/ }`. Angular JIT's
        // `isDelegateCtor` regex only inherits the parent's DI params when the
        // ctor is empty + delegates via `super(...arguments)` (the tsc shape),
        // so rewrite that synthesized form to match.
        {
            let scoping = SemanticBuilder::new()
                .build(&program)
                .semantic
                .into_scoping();
            let mut delegate = DelegateCtorTransform::new();
            traverse_mut(&mut delegate, &allocator, &mut program, scoping, ());
        }

        // CJS mode: rewrite ESM import/export to CommonJS, matching TypeScript's
        // `esModuleInterop` emit. Returns the interop helper prelude text.
        if !options.is_esm() {
            let result = esm_to_cjs::esm_to_cjs(&allocator, &mut program);
            cjs_prelude = result.prelude;
            // Only an actual ESM→CJS conversion gets the `"use strict";` directive
            // and `__esModule` marker; an already-CommonJS module is left as-is
            // (so we never duplicate its `"use strict";` or re-mark it).
            did_cjs = result.converted;
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

    let (code, prefix_lines) = assemble(did_cjs, &cjs_prelude, &coverage_preamble, ret.code);
    TransformResult {
        code,
        source_map: ret
            .map
            .map(|map| finish_source_map(map, prefix_lines, options.source_map_names)),
        coverage_map,
        errors,
    }
}

/// istanbul's `_coverageSchema` magic value (`istanbul-lib-coverage`'s schema hash).
const COVERAGE_SCHEMA_MAGIC: &str = "1a1c01bbd47fc00a2c39e90264f33305004495a9";
const COVERAGE_DATA_ANCHOR: &str = "var coverageData = {";

/// Splice istanbul's `_coverageSchema` marker into the head of the preamble's
/// `coverageData` object literal, as a **bare-identifier** key.
///
/// `istanbul-lib-instrument`'s `readInitialCoverage` — which jest's
/// `generateEmptyCoverage` uses to report never-imported `collectCoverageFrom`
/// files as 0% — locates the coverage object by an `ObjectProperty` whose key is
/// the *identifier* `_coverageSchema` with this exact value. Upstream's preamble
/// embeds the `FileCoverage` JSON without it, so those files would be dropped
/// from the report. The result is a mixed quoted/unquoted-key literal: valid JS,
/// never re-parsed as JSON. Not upstream as of `oxc_coverage_instrument` 0.20.0.
fn with_coverage_schema(preamble: &str) -> String {
    debug_assert!(
        preamble.contains(COVERAGE_DATA_ANCHOR),
        "upstream preamble shape changed: `{COVERAGE_DATA_ANCHOR}` not found, so the \
         `_coverageSchema` marker would be silently dropped"
    );
    preamble.replacen(
        COVERAGE_DATA_ANCHOR,
        &format!("{COVERAGE_DATA_ANCHOR}_coverageSchema:\"{COVERAGE_SCHEMA_MAGIC}\","),
        1,
    )
}

/// Prepend, in order: `"use strict";` + the CJS interop prelude (CJS only), then
/// the coverage preamble (when instrumenting). Returns the assembled code and the
/// number of prepended lines, so the source map can be shifted to match (see
/// [`offset_source_map`]).
fn assemble(
    did_cjs: bool,
    cjs_prelude: &str,
    coverage_preamble: &str,
    code: String,
) -> (String, usize) {
    let mut prefix = String::new();
    // `"use strict";` only for an actual ESM→CJS conversion (ESM is implicitly
    // strict; an already-CommonJS module keeps its own directive, if any). The
    // interop helper prelude is prepended whenever present — including the rare
    // already-CJS module that only needed a dynamic `import()` rewrite.
    if did_cjs {
        prefix.push_str("\"use strict\";\n");
    }
    prefix.push_str(cjs_prelude);
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

/// Serialize codegen's source map, shifting every mapping down by `lines`
/// generated lines and, unless `keep_names`, dropping `names`.
///
/// We prepend `"use strict";` + interop helpers (and the coverage preamble) as
/// raw text after codegen, so the generated code starts `lines` rows lower than
/// the map (built against the codegen output) assumes. Both adjustments are
/// done on the tokens before the single VLQ encode — no JSON re-parse, and no
/// JS-side `mappings` rewrite in the jest plugin. Dropping a name clears only
/// the token's name id; every position is preserved. `token_chunks` (parallel
/// encode hints carrying prev-name ids) are discarded so the encoder works from
/// the rewritten tokens alone. No-op fast path when there is nothing to do.
fn finish_source_map(map: oxc_sourcemap::SourceMap<'_>, lines: usize, keep_names: bool) -> String {
    if lines == 0 && keep_names {
        return map.to_json_string();
    }
    let lines = u32::try_from(lines).unwrap_or(u32::MAX);
    let mut parts = map.into_parts();
    parts.tokens = parts
        .tokens
        .iter()
        .map(|t| {
            oxc_sourcemap::Token::new(
                t.get_dst_line() + lines,
                t.get_dst_col(),
                t.get_src_line(),
                t.get_src_col(),
                t.get_source_id(),
                if keep_names { t.get_name_id() } else { None },
            )
        })
        .collect();
    parts.token_chunks = None;
    if !keep_names {
        parts.names.clear();
    }
    oxc_sourcemap::SourceMap::from_parts(parts).to_json_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    use oxc_sourcemap::{SourceMap, Token};

    fn sample_map() -> SourceMap<'static> {
        SourceMap::new(
            None,
            vec!["foo".into()],
            None,
            vec!["m.ts".into()],
            vec![],
            vec![
                Token::new(0, 0, 0, 0, Some(0), Some(0)),
                Token::new(0, 4, 0, 6, Some(0), None),
                Token::new(1, 2, 3, 1, Some(0), Some(0)),
            ]
            .into_boxed_slice(),
            None,
        )
    }

    fn positions(json: &str) -> Vec<(u32, u32, u32, u32, Option<u32>)> {
        SourceMap::from_json_string(json)
            .unwrap()
            .get_tokens()
            .map(|t| {
                (
                    t.get_dst_line(),
                    t.get_dst_col(),
                    t.get_src_line(),
                    t.get_src_col(),
                    t.get_name_id(),
                )
            })
            .collect()
    }

    #[test]
    fn finish_shifts_lines_without_touching_source_deltas() {
        let out = finish_source_map(sample_map(), 4, true);
        let value: serde_json::Value = serde_json::from_str(&out).unwrap();
        // 4 leading empty lines, then the original mappings unchanged.
        let plain = sample_map().to_json_string();
        let plain: serde_json::Value = serde_json::from_str(&plain).unwrap();
        assert_eq!(
            value["mappings"].as_str().unwrap(),
            format!(";;;;{}", plain["mappings"].as_str().unwrap())
        );
        assert_eq!(value["names"], serde_json::json!(["foo"]));
    }

    #[test]
    fn finish_is_a_no_op_with_nothing_to_do() {
        assert_eq!(
            finish_source_map(sample_map(), 0, true),
            sample_map().to_json_string()
        );
    }

    #[test]
    fn finish_drops_names_but_keeps_positions() {
        let out = finish_source_map(sample_map(), 2, false);
        let value: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(value["names"], serde_json::json!([]));
        // Every segment is 4 VLQ fields now: no 5th (name) field anywhere.
        let mappings = value["mappings"].as_str().unwrap();
        assert_eq!(mappings, ";;AAAA,IAAM;EAGL");
        assert_eq!(
            positions(&out),
            vec![(2, 0, 0, 0, None), (2, 4, 0, 6, None), (3, 2, 3, 1, None)]
        );
    }

    #[test]
    fn source_map_names_false_matches_names_true_positions() {
        // The CJS rewrite of an imported binding (`helper` → `h_1.helper`) is
        // what makes oxc record a name; unrenamed identifiers get none.
        let src = "import { helper } from './h';\nexport const x = helper();\n";
        let base = TransformOptions {
            module: ModuleKind::CommonJs,
            jit_transforms: false,
            ..TransformOptions::default()
        };
        let with = transform(src, "m.ts", &base).source_map.unwrap();
        let without = transform(
            src,
            "m.ts",
            &TransformOptions {
                source_map_names: false,
                ..base
            },
        )
        .source_map
        .unwrap();
        let with_value: serde_json::Value = serde_json::from_str(&with).unwrap();
        assert!(
            !with_value["names"].as_array().unwrap().is_empty(),
            "sanity: oxc emits names by default"
        );
        let without_value: serde_json::Value = serde_json::from_str(&without).unwrap();
        assert_eq!(without_value["names"], serde_json::json!([]));
        let strip = |v: Vec<(u32, u32, u32, u32, Option<u32>)>| {
            v.into_iter()
                .map(|(a, b, c, d, _)| (a, b, c, d))
                .collect::<Vec<_>>()
        };
        let without_positions = positions(&without);
        assert!(without_positions.iter().all(|t| t.4.is_none()));
        assert_eq!(strip(positions(&with)), strip(without_positions));
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
