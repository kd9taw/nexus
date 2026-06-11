# Building & running Nexus on Windows

Nexus's modem (`libft1`) is **Fortran + C/C++ + FFTW**, so the Windows build uses
the **GNU toolchain** (MSVC has no Fortran). This is the same MinGW environment
used to cross-compile FT1 for Windows. The Tauri shell uses the built-in
**WebView2** runtime, audio uses **WASAPI** (via `cpal`, no extra libs), and
PTT/CAT uses Hamlib's **`rigctld.exe`** over TCP.

Nexus has two waveform tiers (the Tempo chat layer protocols), switched with the **Fast · Robust** toggle (never
silently — the active tier is always shown):
- **Fast = FT1** — 4 s T/R, coherent; regional NVIS / good-condition national /
  Field Day / conversational.
- **Robust = DX1** — 15 s T/R, non-coherent 8-FSK; fading-resilient national
  reach (≈3.7 dB fading penalty in simulation vs FT8's 10+ dB collapse). Both
  tiers carry the same messages, so Chat / QSO / Field Day work on either.

## 1. Install the toolchain (MSYS2 + UCRT64)

Install [MSYS2](https://www.msys2.org/), open the **MSYS2 UCRT64** shell, and:

```bash
pacman -S --needed \
  mingw-w64-ucrt-x86_64-gcc \
  mingw-w64-ucrt-x86_64-gcc-fortran \
  mingw-w64-ucrt-x86_64-cmake \
  mingw-w64-ucrt-x86_64-ninja \
  mingw-w64-ucrt-x86_64-fftw \
  mingw-w64-ucrt-x86_64-boost \
  mingw-w64-ucrt-x86_64-pkgconf
```

(These provide `gfortran`, `gcc`, `cmake`, `ninja`, FFTW3 single precision, the
header-only Boost CRC, and `pkg-config`. The `quadmath`/`gfortran` runtime DLLs
come with the gcc-fortran package.)

## 2. Install Rust (GNU target) + Node + WebView2

- Rust via [rustup], then select the GNU host:
  ```bash
  rustup default stable-x86_64-pc-windows-gnu
  ```
  (Must be **`-gnu`**, not `-msvc`, so it links the MinGW gfortran/FFTW/Boost.)
- [Node.js LTS](https://nodejs.org/) (for the web UI build).
- The Tauri CLI: `cargo install tauri-cli --version "^2"`.
- **WebView2 runtime**: preinstalled on Windows 11; on Windows 10 install the
  Evergreen runtime from Microsoft.

Build everything from the **MSYS2 UCRT64** shell so `gfortran`, `cmake`, `ninja`,
FFTW and Boost are on `PATH` (this is what `ft1-sys/build.rs` invokes).

## 3. Build & run

**One-shot:** from the **MSYS2 UCRT64** shell, `./scripts/build-windows.sh` does
steps 1–3 (toolchain check/install, icons, UI deps) and the release build for
you. From Windows PowerShell (it finds MSYS2 and runs the above inside UCRT64):
`./scripts/build-windows.ps1`  — add `--no-radio`, `--dev`, or `--check`. The
manual steps below are what that script automates.

```bash
cd nexus
# Web UI deps (once):
npm --prefix ui install

# Run the desktop app with the live radio loop (sound card + rig):
cargo tauri dev   --features radio       # from src-tauri/, or: cargo tauri dev -d src-tauri ...
# Production build:
cargo tauri build --features radio
```

Without `--features radio` the app builds and runs the full UI but does not key
the radio (useful for trying the interface). The headless workspace
(`cargo test` in `tempo/`) does **not** need WebView2 and runs the modem/engine
tests.

## 4. Configure your station (in-app Settings)

Settings persist to `%APPDATA%\tempo\settings.json`:

- **Callsign / grid** — your identity (used for CQ/beacons and exchanges).
- **Band / dial frequency / sideband** — e.g. 14.074 MHz USB.
- **Field Day class / section** — e.g. `1D` / `WI`.
- **Rig / PTT** — pick a **PTT method** and (for CAT) a **rig model** + **COM
  port** + **baud**. See below.
- **Network** — WSJT-X UDP API + PSK Reporter (see §6).

### Rig control (CAT / PTT)

Nexus handles rig control in-app — **you do not run rigctld yourself**:

- **CAT (recommended):** choose your **rig model** (dropdown) and **COM port** +
  **baud** in Settings. Nexus launches Hamlib's `rigctld` for you and keys/tunes
  the rig. **The installer bundles `rigctld` + its DLLs** (staged by
  `scripts/fetch-hamlib.sh` into `src-tauri/resources/hamlib/`), so installer
  users need **no separate Hamlib install** — Nexus prefers the bundled copy and
  only falls back to a `rigctld` on `PATH`. (If you run a *from-source* build that
  skips the Hamlib fetch, put Hamlib's `rigctld.exe` on `PATH`.) The curated model
  list is best-effort — confirm your exact model number with `rigctl -l` if needed.
- **Serial RTS / DTR:** PTT-only rigs/interfaces — pick the COM port and the
  control line. (Enabled by the `serial` feature, which the `radio` build turns on.)
- **VOX:** no CAT; the rig keys on transmit audio.

## 5. Audio wiring

Point Windows' default **recording** device at the rig's receive audio and the
default **playback** device at the rig's data/mic input (a USB CODEC such as a
SignaLink or the rig's built-in USB audio). `cpal` uses the system default
devices; Tempo resamples to/from the modem's 12 kHz automatically.

## 6. Network: WSJT-X UDP API + PSK Reporter

Enable these in Settings → Network for ecosystem compatibility:

- **WSJT-X UDP API** — Nexus emits the WSJT-X-compatible UDP protocol
  (Heartbeat / Status / Decode / QSO-Logged) so **JTAlert, GridTracker, N1MM and
  loggers** can consume Nexus's decodes/QSOs and (where supported) control it.
  Default target `127.0.0.1:2237` (same as WSJT-X). For another machine on the
  LAN, set its address (and allow the UDP port through Windows Firewall). It also
  accepts inbound Reply / HaltTx / FreeText.
- **PSK Reporter** — uploads your heard stations (call / freq / mode / SNR) to
  `report.pskreporter.info:4739` so your reception shows on the global maps.
  Outbound UDP — allow it through the firewall.

## Notes / troubleshooting

- If CMake picks the wrong generator, ensure you're in the **UCRT64** shell;
  `build.rs` selects Ninja (or MinGW Makefiles) automatically on Windows.
- Link errors about `gfortran`/`quadmath`/`fftw3f`/`stdc++` mean the MSYS2
  packages above aren't on `PATH` — open the UCRT64 shell.
- Time sync matters for decoding: keep the PC clock accurate (Windows time
  service, or a GPS/NTP source for off-grid).
