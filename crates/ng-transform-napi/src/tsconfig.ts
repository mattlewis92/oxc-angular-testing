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
  0: 'es5', // ES3 (oxc has no es3; es5 is the floor)
  1: 'es5',
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
  if (configFile.error) return {};
  const parsed = ts.parseJsonConfigFileContent(
    configFile.config,
    ts.sys,
    path.dirname(resolved),
  );
  const co = parsed.options || {};

  // module: CommonJS ⇒ commonjs; anything else (ES2015+, Node16, NodeNext,
  // ESNext, Preserve) ⇒ esm.
  const moduleKind: 'commonjs' | 'esm' | undefined =
    co.module === undefined
      ? undefined
      : co.module === ts.ModuleKind.CommonJS
        ? 'commonjs'
        : 'esm';

  // useDefineForClassFields default mirrors TS: true when target >= ES2022.
  let useDefine = co.useDefineForClassFields;
  if (useDefine === undefined && co.target !== undefined) {
    useDefine =
      co.target >= ts.ScriptTarget.ES2022 && co.target !== ts.ScriptTarget.JSON;
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
