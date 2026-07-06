#!/usr/bin/env bash
# Fetch the Hamlib (rigctld) Windows runtime and stage it as a Tauri bundle
# resource at src-tauri/resources/hamlib/, so the Windows installer ships CAT
# rig control with zero extra installs. Hamlib is GPL/LGPL — compatible with
# Tempo's GPLv3. The binaries are NOT committed (see .gitignore); this script
# reproduces them.
#
#   ./scripts/fetch-hamlib.sh        # idempotent; skips if already staged
set -euo pipefail

VER=4.7.1
ZIP="hamlib-w64-${VER}.zip"
URL="https://github.com/Hamlib/Hamlib/releases/download/${VER}/${ZIP}"
SHA256=5b2a5d6efc37171c24ee6ac44e6304710219859f30fa4dfc77688f71b3440402

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="$REPO/src-tauri/resources/hamlib"
# What rigctld.exe needs at runtime (+ rigctl.exe for `rigctl -l`, + licenses).
WANT=(rigctld.exe rigctl.exe rotctld.exe rotctl.exe libhamlib-4.dll libwinpthread-1.dll libusb-1.0.dll libgcc_s_seh-1.dll)
LIC=(COPYING.txt COPYING.LIB.txt LICENSE.txt AUTHORS.txt)

if [ -f "$DEST/rigctld.exe" ] && [ -f "$DEST/rotctld.exe" ] && [ -f "$DEST/libhamlib-4.dll" ]; then
  echo "Hamlib already staged at $DEST"; exit 0
fi

tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
echo "Downloading Hamlib $VER…"
(command -v curl >/dev/null && curl -fsSL -o "$tmp/$ZIP" "$URL") || wget -qO "$tmp/$ZIP" "$URL"
echo "$SHA256  $tmp/$ZIP" | sha256sum -c - || { echo "checksum mismatch — aborting"; exit 1; }

if command -v unzip >/dev/null; then
  unzip -qo "$tmp/$ZIP" -d "$tmp"
else
  python3 -c "import zipfile,sys; zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])" "$tmp/$ZIP" "$tmp"
fi

root="$tmp/hamlib-w64-${VER}"
mkdir -p "$DEST"
for f in "${WANT[@]}"; do cp "$root/bin/$f" "$DEST/" && echo "  + $f"; done
for f in "${LIC[@]}";  do cp "$root/$f"     "$DEST/" 2>/dev/null || true; done
echo "Hamlib staged → $DEST"
