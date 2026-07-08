#!/usr/bin/env bash
# Tempo — one-shot Windows build.
#
# Run this from the **MSYS2 UCRT64** shell (Start menu → "MSYS2 UCRT64").
# It installs the MinGW toolchain, ensures the Rust GNU toolchain + Node + the
# Tauri CLI, generates the app icons if missing, and builds the Windows desktop
# app (tempo.exe + an installer).
#
#   ./scripts/build-windows.sh             # release build, with live radio (default)
#   ./scripts/build-windows.sh --no-radio  # UI only — runs, but won't key the radio
#   ./scripts/build-windows.sh --dev        # run in dev mode instead of building
#   ./scripts/build-windows.sh --check      # verify the toolchain only; don't build
#
# Prereqs you must install yourself (the script checks for them):
#   • MSYS2            https://www.msys2.org   (then use the UCRT64 shell)
#   • Rust (rustup)    https://rustup.rs
#   • Node.js LTS      https://nodejs.org
#   • WebView2 runtime (built into Win11; Win10: install the Evergreen runtime)
#   • Hamlib (rigctld.exe on PATH) — only if you use CAT rig control
set -euo pipefail

bold() { printf '\n\033[1m%s\033[0m\n' "$*"; }
ok()   { printf '  \033[32m✓\033[0m %s\n' "$*"; }
warn() { printf '  \033[33m!\033[0m %s\n' "$*"; }
die()  { printf '\n\033[31m✗ %s\033[0m\n' "$*" >&2; exit 1; }

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

RADIO=1; DEV=0; CHECK=0
for a in "$@"; do
  case "$a" in
    --no-radio) RADIO=0 ;;
    --dev)      DEV=1 ;;
    --check)    CHECK=1 ;;
    -h|--help)  sed -n '2,30p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) die "unknown option: $a (try --help)" ;;
  esac
done

bold "Tempo · Windows build"
[ "${MSYSTEM:-}" = "UCRT64" ] || warn "MSYSTEM='${MSYSTEM:-unset}' (expected UCRT64). Open the 'MSYS2 UCRT64' shell. Continuing…"

# 1 — MinGW toolchain (idempotent) -------------------------------------------
bold "1/6  MinGW toolchain (pacman)"
if command -v pacman >/dev/null; then
  pacman -S --needed --noconfirm \
    mingw-w64-ucrt-x86_64-gcc \
    mingw-w64-ucrt-x86_64-gcc-fortran \
    mingw-w64-ucrt-x86_64-cmake \
    mingw-w64-ucrt-x86_64-ninja \
    mingw-w64-ucrt-x86_64-fftw \
    mingw-w64-ucrt-x86_64-boost \
    mingw-w64-ucrt-x86_64-pkgconf
else
  warn "no pacman (not in MSYS2?) — skipping; relying on tools already on PATH"
fi
for t in gfortran gcc g++ cmake ninja pkg-config; do
  command -v "$t" >/dev/null || die "'$t' not on PATH — are you in the MSYS2 UCRT64 shell?"
done
ok "gfortran / gcc / cmake / ninja / pkg-config present"

# 2 — Rust (GNU toolchain) ---------------------------------------------------
bold "2/6  Rust (GNU toolchain)"
command -v cargo >/dev/null || die "Rust not found — install from https://rustup.rs, reopen the shell, re-run."
host="$(rustc -vV | sed -n 's/^host: //p')"
case "$host" in
  *windows-gnu) ok "rustc host: $host" ;;
  *)
    warn "rustc host is '$host' — Tempo needs *-windows-gnu so it links MinGW gfortran/FFTW/Boost."
    if command -v rustup >/dev/null; then
      printf '      switching: rustup default stable-x86_64-pc-windows-gnu\n'
      rustup default stable-x86_64-pc-windows-gnu
    else
      die "need a -gnu toolchain and rustup is not available to switch it."
    fi ;;
esac

# 3 — Node + Tauri CLI -------------------------------------------------------
bold "3/6  Node + Tauri CLI"
command -v node >/dev/null || die "Node.js not found — install the LTS from https://nodejs.org, re-run."
command -v npm  >/dev/null || die "npm not found (ships with Node.js)."
ok "node $(node --version)  npm $(npm --version)"
if cargo tauri --version >/dev/null 2>&1; then
  ok "tauri-cli present ($(cargo tauri --version 2>/dev/null | head -1))"
else
  printf '      installing: cargo install tauri-cli --version "^2"\n'
  cargo install tauri-cli --version "^2"
fi

# 4 — App icons (generate if missing) ----------------------------------------
bold "4/6  App icons"
if [ -f "$REPO/src-tauri/icons/icon.ico" ]; then
  ok "icons present"
elif command -v python3 >/dev/null && python3 -c "import PIL" >/dev/null 2>&1; then
  python3 "$REPO/scripts/gen-icons.py" && ok "icons generated (gen-icons.py)"
elif [ -f "$REPO/src-tauri/icons/icon-source.png" ]; then
  cargo tauri icon "$REPO/src-tauri/icons/icon-source.png" && ok "icons generated (cargo tauri icon)"
else
  die "no icons and no way to generate them (need Pillow, or an icon-source.png for 'cargo tauri icon')."
fi

if [ "$CHECK" = 1 ]; then bold "Toolchain OK (--check). Not building."; exit 0; fi

# 5 — Web UI deps + bundled Hamlib (for CAT) ---------------------------------
bold "5/6  Web UI deps + Hamlib"
npm --prefix "$REPO/ui" install
ok "ui dependencies installed (Tauri's beforeBuildCommand runs the build)"
bash "$REPO/scripts/fetch-hamlib.sh"     # bundle rigctld so CAT needs no separate install
ok "Hamlib staged (bundled into the installer)"

# 6 — Build / run ------------------------------------------------------------
cd "$REPO/src-tauri"
if [ "$DEV" = 1 ]; then
  # Dev uses the live dev server (no custom-protocol).
  feat=(); [ "$RADIO" = 1 ] && feat=(--features radio)
  bold "6/6  Launching Tempo (dev) ${feat[*]:-}"
  exec cargo tauri dev "${feat[@]}"
fi
# Release: custom-protocol embeds the UI (fixes the blank "page cannot be
# displayed" screen); radio enables the sound card + rig.
feats="custom-protocol"; [ "$RADIO" = 1 ] && feats="radio,custom-protocol"
bold "6/6  Building Tempo (release) --features $feats"
cargo tauri build --features "$feats"

bold "Done ✓"
echo "  installer : $REPO/src-tauri/target/release/bundle/nsis/Tempo_*_x64-setup.exe (+ msi/)"
echo "  app exe   : $REPO/src-tauri/target/release/tempo.exe"
[ "$RADIO" = 0 ] && warn "Built WITHOUT --features radio — the UI runs but the radio is not keyed."
echo
echo "The installer bundles the WebView2 runtime + Hamlib (rigctld), so it installs"
echo "clean and CAT/VOX/serial-PTT all work. Copy it to the ham PC and run it →"
echo "Settings: callsign/grid, band, rig/PTT, audio devices."
