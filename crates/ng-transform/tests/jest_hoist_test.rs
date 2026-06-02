//! `babel-plugin-jest-hoist` port: `jest.mock()` & friends hoist above imports.
//! Behavioral cases ported from jest's own hoistPlugin tests.

use ng_transform::{ModuleKind, TransformOptions, transform};

/// Hoist in isolation: no TS lowering / ESM→CJS, so imports stay as `import`
/// and we can assert the reordering directly.
fn hoist(src: &str) -> String {
    let opts = TransformOptions {
        module: ModuleKind::CommonJs,
        jit_transforms: false,
        hoist_jest_mock: true,
        lower: false,
        ..TransformOptions::default()
    };
    let out = transform(src, "a.spec.ts", &opts);
    assert!(out.errors.is_empty(), "errors: {:?}", out.errors);
    out.code
}

/// Full CJS lowering (imports → require), to assert hoist lands above requires.
fn hoist_cjs(src: &str) -> String {
    let opts = TransformOptions {
        module: ModuleKind::CommonJs,
        jit_transforms: false,
        hoist_jest_mock: true,
        ..TransformOptions::default()
    };
    let out = transform(src, "a.spec.ts", &opts);
    assert!(out.errors.is_empty(), "errors: {:?}", out.errors);
    out.code
}

fn before(code: &str, a: &str, b: &str) -> bool {
    match (code.find(a), code.find(b)) {
        (Some(i), Some(j)) => i < j,
        _ => false,
    }
}

#[test]
fn hoists_mock_above_import() {
    let code = hoist("import { foo } from './foo';\njest.mock('./foo');\nfoo();\n");
    assert!(
        before(&code, "jest.mock", "import"),
        "jest.mock must precede the import:\n{code}"
    );
}

#[test]
fn hoists_mock_above_require_in_cjs() {
    // After ESM→CJS the import is a `require`; the hoisted mock must precede it.
    let code = hoist_cjs("import { foo } from './foo';\njest.mock('./foo');\nfoo();\n");
    assert!(
        before(&code, "jest.mock(\"./foo\")", "require(\"./foo\")"),
        "jest.mock must precede require:\n{code}"
    );
}

#[test]
fn hoists_automock_above_require_call() {
    // Ported: `require('x'); jest.enableAutomock(); jest.disableAutomock();`
    let code = hoist("require('x');\njest.enableAutomock();\njest.disableAutomock();\n");
    assert!(
        before(&code, "jest.enableAutomock", "require(\"x\")"),
        "{code}"
    );
    assert!(
        before(&code, "jest.disableAutomock", "require(\"x\")"),
        "{code}"
    );
    // Relative order of the two hoisted calls is preserved.
    assert!(before(&code, "enableAutomock", "disableAutomock"), "{code}");
}

#[test]
fn hoists_within_a_block() {
    // Ported: `beforeEach(() => { require('x'); jest.mock('someNode') })`
    let code = hoist("beforeEach(() => {\n  require('x');\n  jest.mock('someNode');\n});\n");
    assert!(
        before(&code, "jest.mock(\"someNode\")", "require(\"x\")"),
        "mock hoisted to top of the arrow body:\n{code}"
    );
}

#[test]
fn hoists_when_jest_is_destructured_from_jest_globals() {
    // Ported: `const { jest } = require('@jest/globals'); jest.mock(...)`
    let code = hoist(
        "import { foo } from './foo';\nconst { jest } = require('@jest/globals');\njest.mock('./foo');\n",
    );
    assert!(before(&code, "jest.mock(\"./foo\")", "import"), "{code}");
}

#[test]
fn hoists_when_jest_is_imported_from_jest_globals() {
    // Ported: `import { jest } from '@jest/globals'; jest.mock(...)`
    let code = hoist(
        "import { jest } from '@jest/globals';\nimport { foo } from './foo';\njest.mock('./foo');\n",
    );
    assert!(
        before(&code, "jest.mock(\"./foo\")", "from \"./foo\""),
        "{code}"
    );
}

#[test]
fn preserves_order_of_multiple_mocks() {
    let code = hoist("import './x';\njest.mock('a');\njest.mock('b');\njest.mock('c');\n");
    assert!(
        before(&code, "jest.mock(\"a\")", "jest.mock(\"b\")"),
        "{code}"
    );
    assert!(
        before(&code, "jest.mock(\"b\")", "jest.mock(\"c\")"),
        "{code}"
    );
    assert!(before(&code, "jest.mock(\"c\")", "import"), "{code}");
}

#[test]
fn does_not_hoist_do_mock() {
    // `jest.doMock` / `jest.dontMock` are intentionally left in place.
    let code = hoist("import { foo } from './foo';\njest.doMock('./foo');\n");
    assert!(
        before(&code, "import", "jest.doMock"),
        "doMock must NOT be hoisted above the import:\n{code}"
    );
}

#[test]
fn hoists_unmock_and_deep_unmock() {
    let code = hoist("import './x';\njest.unmock('a');\njest.deepUnmock('b');\n");
    assert!(before(&code, "jest.unmock(\"a\")", "import"), "{code}");
    assert!(before(&code, "jest.deepUnmock(\"b\")", "import"), "{code}");
}

#[test]
fn keeps_mock_factory_intact() {
    // The factory (2nd arg) is preserved verbatim when hoisting.
    let code = hoist("import { foo } from './foo';\njest.mock('./foo', () => ({ foo: 1 }));\n");
    assert!(code.contains("jest.mock(\"./foo\", () =>"), "{code}");
    assert!(before(&code, "jest.mock", "import"), "{code}");
}
