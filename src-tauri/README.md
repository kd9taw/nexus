# Tempo — Tauri v2 desktop shell

This directory is the **desktop shell** for Tempo. It is a thin Tauri v2 host
that wraps the web UI in [`../ui`](../ui) and exposes the `tempo-app` command
layer (roster, conversations, link/radio state) to that UI over Tauri IPC.

It is a deliberately **standalone Cargo project** (its `Cargo.toml` declares an
empty `[workspace]`), so it is **not** a member of the root `../Cargo.toml`
workspace. That keeps the Rust workspace (`ft1-sys`, `ft1`, `tempo-core`)
building on hosts with no WebView toolchain.

> ⚠️ **This dev box cannot build the desktop app.** It has no `webkit2gtk` and
> no display. Building/running the Tauri shell requires the platform WebView
> runtime (see prerequisites below). The web UI alone, however, runs fine in a
> plain browser via `npm run dev` for design preview — no Rust, no WebView
> needed.

---

## What's here

| File | Purpose |
|------|---------|
| `Cargo.toml` | `tempo` package: bin `tempo` + lib `tempo_lib`; deps `tauri` v2, `serde`, `serde_json`, `tempo-app`. Standalone workspace. |
| `tauri.conf.json` | Tauri v2 config: identity `com.kd9taw.tempo`, product **Tempo**, window 1280×800 (min 900×600), `frontendDist ../ui/dist`, `devUrl http://localhost:5173`. |
| `build.rs` | Runs `tauri_build::build()` (codegen from `tauri.conf.json`). |
| `src/main.rs` | Thin binary entry point → `tempo_lib::run()`. |
| `src/lib.rs` | Managed `AppState` (behind a `Mutex`) + the four `#[tauri::command]`s + `run()`. |
| `capabilities/default.json` | Tauri v2 capability granting the main window the core webview permissions. |
| `icons/` | App icons (generate with `cargo tauri icon`; placeholders / instructions inside). |

## Command surface (Tauri IPC)

All four commands lock the managed `tempo_app::AppState`, call the matching
domain method, and return the shared **camelCase** DTOs as JSON. The mutating
commands return a fresh `AppSnapshot` so the UI updates in one round-trip.

| Command | Args | Returns | Domain call |
|---------|------|---------|-------------|
| `get_snapshot` | — | `AppSnapshot` | `state.snapshot()` |
| `send_message` | `peer: string`, `text: string` | `AppSnapshot` | `state.send_message(peer, text)` |
| `select_peer` | `peer: string` | `AppSnapshot` | `state.select_peer(peer)` |
| `set_tier` | `tier: "FT1"\|"DX1"` | `AppSnapshot` | `state.set_tier(tier)` (FT8 is Phase 2 — rejected) |

From the web UI:

```ts
import { invoke } from "@tauri-apps/api/core";

const snap = await invoke<AppSnapshot>("get_snapshot");
await invoke("send_message", { peer: "K2DEF", text: "73" });
await invoke("select_peer", { peer: "K2DEF" });
await invoke("set_tier", { tier: "DX1" });
```

> Tauri serializes command args as the JSON object keys you pass (`peer`,
> `text`, `tier`) — keep them exactly as named above.

### DTO contract (shared with the UI and `tempo-app`)

These shapes are defined once in `tempo-app` (`crates/tempo-app/src/dto.rs`, each
with `#[serde(rename_all = "camelCase")]`) and serialized through this shell
unchanged. See the doc comment at the top of `src/lib.rs` for the full list
(`Presence`, `Station`, `ChatMessage`, `Conversation`, `LinkState`,
`RadioStatus`, `Spectrum`, `AppSnapshot`).

### `tempo-app` boundary this shell depends on

The commands bind to the real `tempo-app` surface:

```rust
tempo_app::AppState::new(mycall: &str, mygrid: &str) -> AppState
    .snapshot() -> tempo_app::dto::AppSnapshot
    .send_message(peer: &str, text: &str)
    .select_peer(peer: &str)
    .set_tier(tier: tempo_app::dto::Tier)   // Tier::Ft1 | Tier::Dx1  (Ft8 = Phase 2, rejected)
```

`set_tier`'s incoming `"FT1"`/`"DX1"` string is deserialized into the `Tier`
enum (whose variants serde-rename to exactly those strings); `"FT8"` is rejected
with an error (Phase-2 tier, pipeline not wired). The shell seeds the
identity from `DEFAULT_MYCALL`/`DEFAULT_MYGRID` consts in `src/lib.rs` (TODO:
load from settings once the settings view exists). If `tempo-app`'s signatures
change, only the call sites in `src/lib.rs` need editing — the IPC surface and
DTO contract stay fixed.

---

## Prerequisites by OS

You need a recent stable **Rust** toolchain, **Node.js + npm** (to build the
web UI), and the platform **WebView** runtime plus the Tauri CLI.

Install the Tauri CLI once (either works):

```bash
cargo install tauri-cli --version "^2"   # provides `cargo tauri ...`
# or, project-local:  npm i -D @tauri-apps/cli   then  npx tauri ...
```

### Linux (the WebView toolchain this box is missing)

Tauri v2 on Linux needs **webkit2gtk 4.1** and **libsoup 3**, plus the usual
GTK/build deps. On Debian/Ubuntu:

```bash
sudo apt update
sudo apt install -y \
  libwebkit2gtk-4.1-dev \
  libsoup-3.0-dev \
  build-essential curl wget file \
  libxdo-dev libssl-dev \
  libayatana-appindicator3-dev librsvg2-dev
```

Fedora:

```bash
sudo dnf install webkit2gtk4.1-devel openssl-devel libappindicator-gtk3-devel \
  librsvg2-devel gcc gcc-c++ make
```

Arch:

```bash
sudo pacman -S --needed webkit2gtk-4.1 base-devel curl wget file openssl \
  libappindicator-gtk3 librsvg
```

A graphical session (X11 or Wayland) is required to actually open the window.

### Windows

- **WebView2 runtime** — preinstalled on Windows 11 and current Windows 10; on
  older systems install the Evergreen runtime from Microsoft.
- **Microsoft C++ Build Tools** (MSVC) with the Windows SDK.
- Rust via `rustup` (MSVC toolchain) and Node.js + npm.

### macOS

- **Xcode Command Line Tools**: `xcode-select --install`
  (WKWebView ships with macOS — no extra runtime needed).
- Rust via `rustup` and Node.js + npm.

---

## Build & run

Run these from **this `src-tauri` directory** (or pass `--config`/`-C` paths).
The `beforeDevCommand` / `beforeBuildCommand` in `tauri.conf.json` build the web
UI for you (`npm --prefix ../ui run dev|build`), so you don't start Vite
separately.

```bash
# Dev: launches Vite (../ui) + the native window with hot reload.
cargo tauri dev

# Production: builds ../ui, compiles the Rust shell, bundles an installer.
cargo tauri build
```

Artifacts land under `target/release/bundle/` (`.deb`/`.AppImage`/`.rpm` on
Linux, `.msi`/`.exe` on Windows, `.app`/`.dmg` on macOS).

### App icons

Bundling expects the icons listed in `tauri.conf.json` (`icons/32x32.png`,
`icons/128x128.png`, `icons/128x128@2x.png`, `icons/icon.icns`,
`icons/icon.ico`). Generate the full set from one square PNG (≥1024×1024):

```bash
cargo tauri icon path/to/tempo-logo.png
```

See `icons/README.md` for details. Until icons exist, `cargo tauri build`'s
bundling step will fail on the missing files (compilation itself is unaffected).

---

## Web UI standalone (no Rust / no WebView)

For pure design/UX work the UI runs in any browser against a mock backend that
emits the **same** DTOs:

```bash
npm --prefix ../ui install
npm --prefix ../ui run dev   # open http://localhost:5173
```

The UI's data-access layer detects whether it's inside Tauri (`window.__TAURI__`)
and calls `invoke(...)` if so, otherwise it falls back to the in-browser mock —
so the exact same components render in both environments.

---

## Live radio (sound card + PTT) — `--features radio`

The desktop app drives the rig through the `tempo-audio` crate: 12 kHz audio via
`cpal` and PTT/CAT via Hamlib's **`rigctld`** over TCP (no `libhamlib` build
dependency).

1. Wire the rig's data audio to a sound card (or a CODEC like a SignaLink /
   built-in USB codec).
2. For CAT keying, run rigctld (or use VOX and skip this):

   ```bash
   rigctld -m <rig-model> -r /dev/ttyUSB0 -t 4532    # listens on 127.0.0.1:4532
   ```

   (`RadioConfig::default()` uses `127.0.0.1:4532`, 14.074 MHz, USB. VOX is also
   supported — see `tempo_audio::rig::PttMode`.)
3. Build/run with the radio loop enabled (also needs the audio dev libs:
   `libasound2-dev` on Linux):

   ```bash
   cargo tauri dev --features radio
   # or: cargo tauri build --features radio
   ```

Without `--features radio` the app builds and serves state but does not touch the
radio (useful for UI work). The radio loop runs on its own thread and shares the
engine with the UI via `Arc<Mutex<Engine>>`; on each FT1 slot it transmits the
engine's `poll_tx` audio (holding PTT for the over) or decodes the captured frame
into the engine — which the UI then renders.
