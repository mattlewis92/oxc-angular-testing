#!/usr/bin/env bash
# Symlink the locally-built @oxc-angular-testing packages into a target repo for
# testing, without publishing. Run `pnpm build` in this repo first.
#
# Usage: ./scripts/link-local.sh /path/to/your/angular/repo
#
# Uses plain symlinks (not `pnpm link`) so it doesn't touch the global pnpm
# store and so jest/vitest's `workspace:*` dependency on the transform resolves
# through this monorepo's own node_modules.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="${1:?Usage: link-local.sh <target-repo>}"

if [ ! -f "$REPO/crates/ng-transform-napi/index.js" ]; then
  echo "Native binding not built. Run 'pnpm build' first." >&2
  exit 1
fi

DEST="$TARGET/node_modules/@oxc-angular-testing"
mkdir -p "$DEST"
ln -sfn "$REPO/crates/ng-transform-napi" "$DEST/transform"
ln -sfn "$REPO/packages/jest" "$DEST/jest"
ln -sfn "$REPO/packages/vitest" "$DEST/vitest"

echo "Linked @oxc-angular-testing/{transform,jest,vitest} into:"
echo "  $DEST"
echo
echo "Next: ensure the target repo has '@oxc-project/runtime' installed —"
echo "the lowered decorator code imports it at runtime:"
echo "  (cd \"$TARGET\" && pnpm add -D @oxc-project/runtime)"
