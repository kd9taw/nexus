#!/usr/bin/env bash
# Nexus — native Linux build (Ubuntu/Debian), producing a .deb + AppImage for SourceForge.
#
#   ./scripts/build-linux.sh            # UI + native modem + Tauri .deb/AppImage
#   ./scripts/build-linux.sh --no-gui   # native modem test exes only (fast)
#
# One-time dev deps (Ubuntu 24.04; the script checks and names anything missing):
#   sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev \
#     librsvg2-dev libxdo-dev libssl-dev libsoup-3.0-dev patchelf build-essential \
#     curl wget file cmake ninja-build gfortran nodejs npm
#   + rustup (https://rustup.rs); cargo-tauri is auto-installed if absent.
#
# Native build uses the SYSTEM FFTW3f (libfftw3f-dev) via pkg-config — no cross FFTW needed.
# CAT on Linux uses the system Hamlib: the .deb depends on libhamlib-utils (rigctld), and the
# AppImage falls back to `rigctld` on PATH, so AppImage users run `sudo apt install libhamlib-utils`.
set -euo pipefail

bold() { printf '\n\033[1m%s\033[0m\n' "$*"; }
ok()   { printf '  \033[32m✓\033[0m %s\n' "$*"; }
warn() { printf '  \033[33m!\033[0m %s\n' "$*"; }
die()  { printf '\n\033[31m✗ %s\033[0m\n' "$*" >&2; exit 1; }

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# Official-build secrets (e.g. CLUBLOG_API_KEY, baked via option_env!) live OUTSIDE the repo.
# shellcheck disable=SC1091
[ -f "$HOME/.nexus-build.env" ] && source "$HOME/.nexus-build.env"

GUI=1
for a in "$@"; do
  case "$a" in
    --no-gui) GUI=0 ;;
    -h|--help) sed -n '2,20p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) die "unknown option: $a" ;;
  esac
done

# 1 — toolchain + the GTK/WebKit dev libraries the native Tauri build links against ----------
bold "1/4  Toolchain + Linux GUI dev libraries"
miss=()
for t in cc gfortran cmake node npm; do command -v "$t" >/dev/null || miss+=("$t"); done
command -v ninja >/dev/null || command -v make >/dev/null || miss+=("ninja-or-make")
[ "$GUI" = 1 ] && { command -v patchelf >/dev/null || miss+=("patchelf"); }
[ "${#miss[@]}" -eq 0 ] || die "missing tools: ${miss[*]}
  Ubuntu/Debian: sudo apt install build-essential cmake ninja-build gfortran nodejs npm patchelf"
command -v cargo >/dev/null || die "Rust not found — install from https://rustup.rs"
pkg-config --exists fftw3f 2>/dev/null || die "libfftw3f-dev missing — sudo apt install libfftw3-dev"
if [ "$GUI" = 1 ]; then
  pcmiss=()
  for pc in webkit2gtk-4.1 gtk+-3.0 librsvg-2.0; do
    pkg-config --exists "$pc" 2>/dev/null || pcmiss+=("$pc")
  done
  [ "${#pcmiss[@]}" -eq 0 ] || die "missing GUI dev libraries (pkg-config): ${pcmiss[*]}
  Ubuntu/Debian: sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev libsoup-3.0-dev"
fi
GEN=Ninja; command -v ninja >/dev/null || GEN="Unix Makefiles"
ok "cc/gfortran/cmake ($GEN)/node, system FFTW3f$([ "$GUI" = 1 ] && echo ', webkit2gtk-4.1, patchelf')"

# The DeepCW AI CW model (AGPL-3.0, (c) e04) is NOT committed — it is gitignored and staged
# into src-tauri/resources/deepcw by the caller (see that folder's README.md). Tauri's resource
# glob matches the directory whether or not the model is in it, so a missing model bundles a
# .deb that installs fine, runs fine, and silently has no AI CW decoder — 14 MB lighter with no
# error anywhere. A CI checkout has no way to obtain the file, so this is the DEFAULT there,
# not an edge case. Fail loudly instead of shipping a quietly-lobotomised build.
if [ "$GUI" = 1 ]; then
  dcw="$REPO/src-tauri/resources/deepcw"
  for f in model.onnx model.onnx.json; do
    [ -s "$dcw/$f" ] || die "missing $dcw/$f — the DeepCW model is gitignored and must be staged
  before bundling, or the build silently ships without the AI CW decoder.
  See src-tauri/resources/deepcw/README.md for provenance and how to fetch/fold it."
  done
  ok "DeepCW model staged ($(du -h "$dcw/model.onnx" | cut -f1))"
fi

# 2 — libft1 native modem test exes (proves the native chain; system FFTW3f via pkg-config) --
bold "2/4  libft1 native modem test exes"
# WX selects the WSJT-X-derived modem source. Unset (the normal case) means the in-tree
# vendored copy at libft1/vendor/wsjtx. Export WX=/path/to/wsjtx-source to build against a
# different checkout; ft1-sys/build.rs reads the same variable, so both stay in step.
cmake -S "$REPO/libft1" -B "$REPO/libft1/build-linux" -G "$GEN" -DCMAKE_BUILD_TYPE=Release \
  ${WX:+-DWX="$WX"} >/dev/null
cmake --build "$REPO/libft1/build-linux" >/dev/null
for e in dx1_test_standalone roundtrip ft1_test_standalone acquire; do
  [ -f "$REPO/libft1/build-linux/$e" ] && ok "$e" || warn "$e not produced"
done

if [ "$GUI" = 0 ]; then bold "Modem exes done (--no-gui)."; exit 0; fi

# 3 — UI build deps ---------------------------------------------------------------------------
bold "3/4  Web UI dependencies"
( cd "$REPO/ui" && npm install >/dev/null )
ok "ui/node_modules"

# 4 — the GUI app + offline .deb + AppImage ---------------------------------------------------
bold "4/4  Nexus GUI app + .deb + AppImage"
cargo tauri --version >/dev/null 2>&1 || { warn "installing tauri-cli…"; cargo install tauri-cli --version "^2" --locked; }
[ -f "$REPO/src-tauri/icons/128x128.png" ] || python3 "$REPO/scripts/gen-icons.py"
# Linux uses the SYSTEM Hamlib (rigctld on PATH / the .deb's libhamlib-utils dependency), so DON'T
# ship the Windows hamlib .dll/.exe in the Linux bundle. The Windows build re-stages the real
# binaries via fetch-hamlib.sh, so removing them here is safe.
#
# Remove ONLY the untracked Windows binaries. This used to `rm -rf` the whole directory and
# recreate it with just a README — which DELETED four TRACKED Hamlib license files
# (AUTHORS/COPYING/COPYING.LIB/LICENSE) on every run, leaving the working tree dirty and, worse,
# one `git add -A` away from committing the removal of the license texts Hamlib's LGPL requires
# us to ship. The tracked README.txt is byte-identical to what that heredoc wrote, so nothing
# was gained by recreating it. Bit us for real on 2026-07-20.
find "$REPO/src-tauri/resources/hamlib" -type f \
  \( -name '*.dll' -o -name '*.exe' -o -name '*.lib' -o -name '*.def' \) -delete
git -C "$REPO" diff --quiet -- src-tauri/resources/hamlib || \
  die "build-linux.sh modified TRACKED files under src-tauri/resources/hamlib — refusing to
  continue. Those are the LGPL license texts Hamlib requires us to distribute; restore with
  'git checkout -- src-tauri/resources/hamlib/'."
( cd "$REPO/src-tauri" && cargo tauri build --features radio,custom-protocol --bundles deb,appimage )
ok "Nexus .deb + AppImage"

bold "Done ✓  Linux artifacts:"
echo "  .deb     : src-tauri/target/release/bundle/deb/*.deb"
echo "  AppImage : src-tauri/target/release/bundle/appimage/*.AppImage"
echo "  binary   : src-tauri/target/release/nexus"
echo
warn "CAT needs Hamlib: the .deb pulls libhamlib-utils automatically; AppImage users run"
warn "'sudo apt install libhamlib-utils'. FT8/FT4 audio decode works without it (VOX)."
