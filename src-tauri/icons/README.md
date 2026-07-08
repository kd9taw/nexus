# App icons

`tauri.conf.json` references these icon files (all committed, generated from one
design):

- `32x32.png`, `128x128.png`, `128x128@2x.png` — window / Linux icons
- `icon.ico` — Windows (multi-size: 16–256)
- `icon.icns` — macOS
- `icon-source.png` — the 1024×1024 master

To change the icon, edit the design in `scripts/gen-icons.py` and re-run it:

```bash
python3 scripts/gen-icons.py     # needs Pillow (PIL)
```

Or regenerate from any square PNG with the Tauri CLI:

```bash
cargo tauri icon path/to/source.png
```

`cargo tauri build` needs these at bundle time; plain `cargo build`/`cargo check`
does not.
