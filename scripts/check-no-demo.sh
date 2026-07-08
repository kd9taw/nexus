#!/usr/bin/env bash
# Guardrail: no demo / mock / fabricated data may reach an operator.
#
# The app must NEVER serve made-up propagation, sample QSOs, or a browser mock in
# a shipped build. This fails the build if any of the known demo/mock fingerprints
# reappear in shipped code (or in the built UI bundle). The deleted offenders were:
#   - Rust  : propagation::demo() (the fake C91RU/VP8XYZ nowcast) + its re-export
#   - TS    : ui/src/mock.ts (mockEngine / demoPropagation / live demo driver) +
#             DemoBanner, and the silent `else mockEngine.*` fallback in api.ts
#
# Run from the repo's `tempo/` dir:  scripts/check-no-demo.sh
# Optionally also greps ui/dist if it exists (proves the mock tree-shook out).
set -euo pipefail

cd "$(dirname "$0")/.."   # -> tempo/
fail=0
report() { echo "  ✗ FORBIDDEN: $1"; fail=1; }

echo "== no-demo guard =="

# --- Rust: the fabricated nowcast must be gone everywhere (not even behind cfg) ---
# These tokens are never legitimate (the test fixture is rich_fixture(), and the
# offline snapshot uses source="offline"); a single hit means demo data crept back.
if grep -rnE 'propagation::demo|fn +demo *\(|source *[:=] *"demo"' \
    src-tauri/src crates/*/src 2>/dev/null; then
  report "Rust demo() / source=\"demo\" reference"
fi

# --- TS: the mock harness is deleted; nothing may import or reference it ---
if [ -e ui/src/mock.ts ]; then report "ui/src/mock.ts exists (must be deleted)"; fi
if [ -e ui/src/components/DemoBanner.tsx ]; then report "DemoBanner.tsx exists (must be deleted)"; fi
if grep -rnE "mockEngine|demoPropagation|nextSpectrumRow|from '\.\.?/mock'|DemoBanner" \
    ui/src 2>/dev/null; then
  report "TS mock/demo reference in ui/src"
fi

# --- Built bundle: prove the mock dataset tree-shook out of the shipped JS ---
if ls ui/dist/assets/*.js >/dev/null 2>&1; then
  if grep -roiE "demoPropagation|mockEngine|nextSpectrumRow|Demo Operator" ui/dist/assets/*.js 2>/dev/null; then
    report "demo/mock symbol present in the built UI bundle"
  else
    echo "  ✓ ui/dist bundle is demo/mock-free"
  fi
fi

if [ "$fail" -ne 0 ]; then
  echo "FAILED: demo/mock data found in shipped code. No demo data may reach an operator."
  exit 1
fi
echo "  ✓ no demo/mock data in shipped code"
