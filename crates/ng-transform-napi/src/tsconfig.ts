// Derive @oxc-angular-testing/transform options from a project tsconfig, the way
// jest-preset-angular reads the TS compiler options. Uses the `typescript`
// package (resolved from the consumer) so `extends` chains and defaults resolve
// correctly. Returns `{}` if typescript or the config can't be loaded.

import * as path from 'node:path';
import type * as TS from 'typescript';

export interface DerivedTransformOptions {
  target?: string;
  module?: 'commonjs' | 'esm';
  experimentalDecorators?: boolean;
  emitDecoratorMetadata?: boolean;
  useDefineForClassFields?: boolean;
  jsx?: 'automatic' | 'classic';
  jsxImportSource?: string;
  jsxFactory?: string;
  jsxFragmentFactory?: string;
  jsxDevelopment?: boolean;
}

// ts.ScriptTarget enum value → oxc target string.
const SCRIPT_TARGET: Record<number, string> = {
  // oxc's downlevel floor is es2015 ("es5" is explicitly rejected by its
  // EnvOptions::from_target), so ES3/ES5 clamp up to es2015 — the lowest oxc emits.
  0: 'es2015', // ES3
  1: 'es2015', // ES5
  2: 'es2015',
  3: 'es2016',
  4: 'es2017',
  5: 'es2018',
  6: 'es2019',
  7: 'es2020',
  8: 'es2021',
  9: 'es2022',
  10: 'es2023',
  11: 'es2024',
  99: 'esnext',
};

export function scriptTargetToString(target: number): string {
  return SCRIPT_TARGET[target] ?? 'esnext';
}

/**
 * Derive transform options (target, module format, decorator flags,
 * `useDefineForClassFields`) from a project tsconfig. Requires `typescript` to
 * be resolvable; returns `{}` otherwise.
 */
export function deriveTransformOptions(
  tsconfigPath: string,
  cwd: string = process.cwd(),
): DerivedTransformOptions {
  let ts: typeof TS;
  try {
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    ts = require('typescript') as typeof TS;
  } catch {
    return {};
  }
  const resolved = path.isAbsolute(tsconfigPath)
    ? tsconfigPath
    : path.resolve(cwd, tsconfigPath);
  const configFile = ts.readConfigFile(resolved, ts.sys.readFile);
  if (configFile.error) {
    // A tsconfig was explicitly requested but can't be read — missing file or
    // malformed JSON. Silently returning {} would drop every derived option
    // (target/module/decorator flags) and fall back to oxc's defaults (esnext, no
    // decorator metadata, …): a silent miscompile. This is a misconfiguration we
    // refuse to skip — throw with the real TS diagnostic. (A common cause is an
    // unexpanded jest `<rootDir>` token reaching here — the jest plugin expands it,
    // but a custom wiring may not.)
    const detail = ts.flattenDiagnosticMessageText(configFile.error.messageText, '\n');
    throw new Error(
      `@oxc-angular-testing: could not read tsconfig "${resolved}": ${detail} ` +
        `If the path contains an unexpanded "<rootDir>" token, resolve it before it reaches the transform.`,
    );
  }
  const parsed = ts.parseJsonConfigFileContent(
    configFile.config,
    ts.sys,
    path.dirname(resolved),
  );
  // parseJsonConfigFileContent returns best-effort options even when the config
  // mis-parses (bad `extends`, invalid option values). Those land in `parsed.errors`
  // and were previously ignored, yielding partial options silently. A referenced
  // tsconfig that doesn't parse is a misconfiguration — fail loudly.
  const parseErrors = parsed.errors.filter((d) => d.category === ts.DiagnosticCategory.Error);
  if (parseErrors.length > 0) {
    const detail = parseErrors
      .map((d) => ts.flattenDiagnosticMessageText(d.messageText, '\n'))
      .join('; ');
    throw new Error(`@oxc-angular-testing: tsconfig "${resolved}" has errors: ${detail}`);
  }
  const co = parsed.options || {};

  // module: CommonJS ⇒ commonjs; anything else (ES2015+, Node16, NodeNext,
  // ESNext, Preserve) ⇒ esm.
  const moduleKind: 'commonjs' | 'esm' | undefined =
    co.module === undefined
      ? undefined
      : co.module === ts.ModuleKind.CommonJS
        ? 'commonjs'
        : 'esm';

  // useDefineForClassFields default mirrors TS: true when the EFFECTIVE target is
  // >= ES2022. Use ts.getEmitScriptTarget so a tsconfig that omits `target` but sets
  // a modern `module` (node16/nodenext/esnext → effective target ES2022+) resolves
  // the same way tsc does, instead of falling back to the Rust default (false).
  let useDefine = co.useDefineForClassFields;
  if (useDefine === undefined) {
    // `ts.getEmitScriptTarget` resolves the effective target (applying module-based
    // defaults like node16/nodenext → ES2022+). It is exported at runtime but marked
    // `@internal` (absent from the public types), so reach it via a cast and fall
    // back to the explicit target (or the ES5 floor) if a future TS drops it.
    const getEmitScriptTarget = (
      ts as unknown as {
        getEmitScriptTarget?: (o: TS.CompilerOptions) => TS.ScriptTarget;
      }
    ).getEmitScriptTarget;
    const effective = getEmitScriptTarget
      ? getEmitScriptTarget(co)
      : (co.target ?? ts.ScriptTarget.ES5);
    useDefine = effective >= ts.ScriptTarget.ES2022;
  }

  const options: DerivedTransformOptions = {};
  if (co.target !== undefined) options.target = scriptTargetToString(co.target);
  if (moduleKind !== undefined) options.module = moduleKind;
  if (co.experimentalDecorators !== undefined) {
    options.experimentalDecorators = co.experimentalDecorators;
  }
  if (co.emitDecoratorMetadata !== undefined) {
    options.emitDecoratorMetadata = co.emitDecoratorMetadata;
  }
  if (useDefine !== undefined) options.useDefineForClassFields = useDefine;

  // JSX (mixed Angular + React). ts.JsxEmit: React=2 (classic), ReactJSX=4,
  // ReactJSXDev=5 (automatic); Preserve=1 / ReactNative=3 → automatic so the
  // .tsx is still runnable under the test runner.
  if (co.jsx !== undefined) {
    if (co.jsx === ts.JsxEmit.React) {
      options.jsx = 'classic';
      if (co.jsxFactory) options.jsxFactory = co.jsxFactory;
      if (co.jsxFragmentFactory) options.jsxFragmentFactory = co.jsxFragmentFactory;
    } else {
      options.jsx = 'automatic';
      options.jsxDevelopment = co.jsx === ts.JsxEmit.ReactJSXDev;
      if (co.jsxImportSource) options.jsxImportSource = co.jsxImportSource;
    }
  }
  return options;
}
