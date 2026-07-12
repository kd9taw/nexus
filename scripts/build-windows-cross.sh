#!/usr/bin/env bash
# Tempo — cross-build the Windows app from Linux / WSL2 (no MSYS2 needed).
#
# Uses the MinGW-w64 cross toolchain to produce Windows .exe files right here:
#   • src-tauri/target/x86_64-pc-windows-gnu/release/Nexus.exe        (the GUI app)
#   • target/x86_64-pc-windows-gnu/release/examples/win_smoke.exe     (static modem self-test)
#   • libft1/build-win/{dx1_test_standalone,roundtrip,ft1_test_standalone,acquire}.exe
#
#   ./scripts/build-windows-cross.sh            # everything (modem exes + GUI)
#   ./scripts/build-windows-cross.sh --modem    # only the modem exes (fast, fully static)
#   ./scripts/build-windows-cross.sh --no-gui   # modem + win_smoke, skip Nexus.exe
#
# Prereqs (Debian/Ubuntu pkg names) — the script checks and reports what's missing:
#   gcc-mingw-w64-x86-64 g++-mingw-w64-x86-64 gfortran-mingw-w64-x86-64
#   cmake ninja-build nodejs npm  +  rustup  +  cc/make (to build FFTW)
set -euo pipefail

bold() { printf '\n\033[1m%s\033[0m\n' "$*"; }
ok()   { printf '  \033[32m✓\033[0m %s\n' "$*"; }
warn() { printf '  \033[33m!\033[0m %s\n' "$*"; }
die()  { printf '\n\033[31m✗ %s\033[0m\n' "$*" >&2; exit 1; }

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# Official-build secrets (e.g. CLUBLOG_API_KEY, baked in via option_env!) live OUTSIDE the
# repo so they're never committed — ClubLog requires the key stay out of the public source.
# shellcheck disable=SC1091
[ -f "$HOME/.nexus-build.env" ] && source "$HOME/.nexus-build.env"
export PATH="$HOME/.local/bin:$PATH"     # picks up a pip-installed ninja, if any
TARGET=x86_64-pc-windows-gnu
FFTW_VER=3.3.10
export FFTW_MINGW_PREFIX="$REPO/target/fftw-mingw"   # read by ft1-sys/build.rs

GUI=1; MODEM_ONLY=0
for a in "$@"; do
  case "$a" in
    --modem)  MODEM_ONLY=1 ;;
    --no-gui) GUI=0 ;;
    -h|--help) sed -n '2,20p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) die "unknown option: $a" ;;
  esac
done

# 1 — toolchain checks -------------------------------------------------------
bold "1/5  Cross toolchain"
miss=()
for t in x86_64-w64-mingw32-gcc x86_64-w64-mingw32-g++ x86_64-w64-mingw32-gfortran cmake; do
  command -v "$t" >/dev/null || miss+=("$t")
done
command -v ninja >/dev/null || command -v make >/dev/null || miss+=("ninja-or-make")
[ "${#miss[@]}" -eq 0 ] || die "missing: ${miss[*]}
  Debian/Ubuntu: sudo apt install gcc-mingw-w64-x86-64 g++-mingw-w64-x86-64 gfortran-mingw-w64-x86-64 cmake ninja-build"
command -v cargo >/dev/null || die "Rust not found — install from https://rustup.rs"
rustup target list --installed 2>/dev/null | grep -qx "$TARGET" || { warn "adding Rust target $TARGET"; rustup target add "$TARGET"; }
GEN=Ninja; command -v ninja >/dev/null || GEN="Unix Makefiles"
ok "mingw gcc/g++/gfortran, cmake ($GEN), Rust target $TARGET"

# 2 — FFTW3f for MinGW (built once, cached) ----------------------------------
bold "2/5  FFTW3f (single precision) for MinGW"
if [ -f "$FFTW_MINGW_PREFIX/lib/libfftw3f.a" ]; then
  ok "cached at $FFTW_MINGW_PREFIX"
else
  tmp="$(mktemp -d)"
  ( cd "$tmp"
    url="http://www.fftw.org/fftw-${FFTW_VER}.tar.gz"
    (command -v curl >/dev/null && curl -fsSL -o fftw.tgz "$url") || wget -qO fftw.tgz "$url" \
      || die "could not download $url"
    tar xf fftw.tgz && cd "fftw-${FFTW_VER}"
    ./configure --host=x86_64-w64-mingw32 --enable-float --enable-static \
      --disable-shared --prefix="$FFTW_MINGW_PREFIX" >/dev/null
    make -j"$(nproc)" >/dev/null && make install >/dev/null )
  rm -rf "$tmp"
  [ -f "$FFTW_MINGW_PREFIX/lib/libfftw3f.a" ] && ok "built → $FFTW_MINGW_PREFIX" || die "FFTW cross-build failed"
fi

# 3 — libft1 modem test exes (proves the native chain on Windows) ------------
bold "3/5  libft1 Windows test exes"
cmake -S "$REPO/libft1" -B "$REPO/libft1/build-win" -G "$GEN" \
  -DCMAKE_TOOLCHAIN_FILE="$REPO/libft1/mingw-w64.cmake" \
  -DFFTW_MINGW_PREFIX="$FFTW_MINGW_PREFIX" -DCMAKE_BUILD_TYPE=Release >/dev/null
cmake --build "$REPO/libft1/build-win" >/dev/null
for e in dx1_test_standalone roundtrip ft1_test_standalone acquire; do
  [ -f "$REPO/libft1/build-win/$e.exe" ] && ok "$e.exe" || warn "$e.exe not produced"
done

# 4 — win_smoke.exe : FT1 + DX1 round-trip, fully static (no DLLs) -----------
bold "4/5  Rust modem self-test (win_smoke.exe)"
cargo build --release --target "$TARGET" -p ft1 --example win_smoke
ok "target/$TARGET/release/examples/win_smoke.exe"

if [ "$MODEM_ONLY" = 1 ]; then bold "Modem exes done (--modem)."; exit 0; fi

# 5 — the GUI app + offline installer (tempo.exe + NSIS) ---------------------
if [ "$GUI" = 1 ]; then
  bold "5/5  GUI app + installer (Nexus.exe, NSIS)"
  command -v npm >/dev/null || die "npm not found — install Node.js LTS to build the UI."
  command -v makensis >/dev/null || warn "makensis not found — the NSIS step needs it (Debian/Ubuntu: sudo apt install nsis)."
  cargo tauri --version >/dev/null 2>&1 || { warn "installing tauri-cli…"; cargo install tauri-cli --version "^2" --locked; }
  ( cd "$REPO/ui" && npm install >/dev/null )       # deps; cargo tauri runs the build
  [ -f "$REPO/src-tauri/icons/icon.ico" ] || python3 "$REPO/scripts/gen-icons.py"
  bash "$REPO/scripts/fetch-hamlib.sh"              # bundle Hamlib for CAT (no-op if staged)
  # cargo tauri build enables asset embedding (custom-protocol — the fix for the
  # blank "page cannot be displayed" screen) and bundles the offline installer.
  ( cd "$REPO/src-tauri" && cargo tauri build --target "$TARGET" --features radio,custom-protocol --bundles nsis )
  ok "Nexus.exe + installer"
fi

bold "Done ✓  Windows artifacts:"
echo "  installer (run this): src-tauri/target/$TARGET/release/bundle/nsis/Nexus_*_x64-setup.exe"
echo "  GUI app             : src-tauri/target/$TARGET/release/Nexus.exe"
echo "  modem self-test     : target/$TARGET/release/examples/win_smoke.exe   (fully static — runs anywhere)"
echo "  modem test exes     : libft1/build-win/*.exe"
echo
warn "Cross-compiled, NOT run here — smoke-test on Windows. The installer bundles"
warn "the offline WebView2 runtime + Hamlib (rigctld), so it installs clean & CAT works."
