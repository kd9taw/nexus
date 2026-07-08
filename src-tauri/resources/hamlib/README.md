# Bundled Hamlib (rigctld) runtime

Tempo bundles Hamlib's `rigctld.exe` + runtime DLLs here so the Windows
installer provides **CAT rig control with no extra installs**. When `rigctld` is
present next to the app, `tempo-audio`'s rig launcher uses it automatically (it
falls back to a `rigctld` on `PATH` otherwise).

These files are **not committed** (~14 MB of DLLs). Fetch them with:

```bash
./scripts/fetch-hamlib.sh      # downloads Hamlib 4.7.1 w64, verifies SHA-256
```

Staged set (the minimal runtime for `rigctld`):

- `rigctld.exe` — the daemon Tempo launches for CAT
- `rigctl.exe` — handy for `rigctl -l` (list rig model numbers)
- `libhamlib-4.dll` — the radio backends (the large one)
- `libwinpthread-1.dll`, `libusb-1.0.dll`, `libgcc_s_seh-1.dll` — its deps
- `COPYING.txt` / `COPYING.LIB.txt` / `LICENSE.txt` / `AUTHORS.txt` — Hamlib is
  GPL/LGPL, compatible with Tempo's GPLv3; these ship in the installer.

The Tauri bundler includes this directory via `bundle.resources` in
`tauri.conf.json`, so it installs alongside `tempo.exe`.
