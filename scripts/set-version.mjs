// Stamp a release version across the publishable package.json files.
//
// Usage: node scripts/set-version.mjs <version>
//
// Only the three published packages are touched. The napi platform packages
// (`@oxc-angular-testing/binding-*`) and the main package's
// `optionalDependencies` are generated/updated from the transform package's
// version by `napi prepublish`, and the jest/vitest `workspace:*` dependency on
// `@oxc-angular-testing/transform` is rewritten to this exact version by
// `pnpm publish` — so setting these three keeps everything in lockstep.

import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const version = process.argv[2];

// Semver (with optional prerelease) — refuse anything that would publish junk.
if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version ?? '')) {
  console.error(`set-version: invalid version ${JSON.stringify(version)} (expected e.g. 1.2.3 or 1.2.3-beta.0)`);
  process.exit(1);
}

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const files = [
  'crates/ng-transform-napi/package.json',
  'packages/jest/package.json',
  'packages/vitest/package.json',
];

for (const rel of files) {
  const path = join(root, rel);
  const pkg = JSON.parse(readFileSync(path, 'utf8'));
  pkg.version = version;
  writeFileSync(path, `${JSON.stringify(pkg, null, 2)}\n`);
  console.log(`set ${pkg.name} -> ${version}`);
}
