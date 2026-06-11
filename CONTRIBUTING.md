# Contributing to Nexus

Thanks for your interest in Nexus — an all-mode amateur radio operations center
(FT8/FT4 digital with WSJT-X parity, CW, SSB phone, propagation intelligence,
logging/awards, POTA/SOTA, Field Day) that also carries the experimental Tempo
FT1/DX1 chat layer. Contributions of all kinds are welcome: code,
documentation, bug reports, and especially **real on-air decode-rate data from
the FT1/DX1 tiers** (more on that below).

Nexus is maintained by Seth McCallister (KD9TAW)
&lt;kd9taw@protonmail.com&gt;. The canonical repo is
<https://github.com/kd9taw/nexus>.

---

## What we value most

The FT8/FT4 tier is in daily production use — it implements the WSJT-X
protocol and is not what needs validation. The **FT1 and DX1 waveforms** are
**validated by simulation only** — AWGN and fading sweeps. In simulation FT1
reaches its 50% decode threshold around **-15 dB** (AWGN), and DX1 around
**-18.6 dB** with only a **~3.7 dB** fading penalty (versus 10+ dB for
FT8-class coherent modes). These numbers have **not yet been confirmed on the
air** — that is the FT1/DX1 tier's hard remaining gate.

So the single most useful thing you can contribute for the FT1/DX1 tiers is
**honest on-air observation**: decode rates, SNR/path notes,
antenna/band/conditions, dupes, sequencer hiccups, and bug reports from real
QSOs. Simulation tells us the modem *should* work; only operators can tell us
whether it *does*. Please open issues with as much detail as you can — band,
dial, mode/tier, distance, conditions, and what you saw versus what you
expected.

Published Windows binaries are **cross-compiled beta** builds. Treat them
accordingly and report what breaks.

---

## How the project is organized

Nexus is a Cargo workspace (Rust 2021) plus a Vite/React/TypeScript UI and a
Fortran/C/C++ modem. The workspace members (`Cargo.toml`) are:

| Crate | Path | Responsibility |
|-------|------|----------------|
| `ft1-sys` | `crates/ft1-sys` | Raw FFI to `libft1`. Its `build.rs` runs CMake to build the modem (cross-aware for `windows-gnu`). |
| `ft1` | `crates/ft1` | Safe wrapper over `libft1`; includes a `dx1` module (DX1 mode, in `src/lib.rs`) and a `win_smoke` example (`examples/win_smoke.rs`). |
| `tempo-core` | `crates/tempo-core` | Protocol/domain logic: slot timing, virtual channel, message, QSO, roster, inbox, store-and-forward, Field Day, spectrum, text chunking, TX. |
| `tempo-app` | `crates/tempo-app` | UI-facing logic: serde DTOs (camelCase, `src/dto.rs`), settings, and the live TX/RX `Engine` (Chat / QSO / Field Day modes). Headless-testable. |
| `tempo-audio` | `crates/tempo-audio` | Real transport: `cpal` sound card (feature `device`), rig control (`rigctld` launch / serial RTS-DTR / VOX, feature `serial`), rig models (`src/rigmodels.rs`), and the slot-clock service loop. |
| `tempo-net` | `crates/tempo-net` | WSJT-X-compatible UDP API (magic `0xADBCCBDA`, schema 3) and PSK Reporter spotting. Pure std Rust. |

Outside the workspace:

- `src-tauri/` — Tauri v2 desktop shell. **Standalone crate with its own
  `[workspace]`**, so it is built separately from the main workspace. Features:
  `radio` (= `device` + `serial`) and `custom-protocol`. `main.rs` calls
  `tempo_lib::run()`.
- `ui/` — Vite + React + TypeScript web UI (npm). The TypeScript DTO contract
  in `ui/src/types.ts` mirrors `tempo-app/src/dto.rs` — keep the two in sync.
- `libft1/` — the modem: Fortran + C + C++, FFTW3 single-precision, **no Qt**.
  `ft1_cabi.f90` is the C ABI; `libft1/dx1/` is the DX1 mode; `mingw-w64.cmake`
  is the cross toolchain file.
- `scripts/` — build and asset scripts (see below).
- `WINDOWS.md` — the authoritative Windows build and setup guide.

---

## Where to put new code

- **Message / protocol types, timing, QSO/Field Day logic** → `tempo-core`.
- **New DTOs or engine behavior exposed to the UI** → `tempo-app` (and mirror
  the DTO in `ui/src/types.ts`).
- **New rig models** → `tempo-audio/src/rigmodels.rs`.
- **Sound card / PTT / CAT transport** → `tempo-audio`.
- **WSJT-X UDP or PSK Reporter changes** → `tempo-net`.
- **UI** → `ui/src`.
- **Modem / waveform changes (FT1, DX1, the C ABI)** → `libft1`, then surface
  through `ft1-sys` (raw) and `ft1` (safe).

When in doubt, match the crate whose responsibility (table above) best fits the
change, and keep the boundary clean — `tempo-core` stays free of I/O,
`tempo-audio` owns the hardware, `tempo-app` glues it together for the UI.

---

## Development environment

### Rust workspace (modem + core + engine + net)

`cargo test --workspace` builds and runs the headless test suite (modem,
engine, net, and DX1 round-trips). Because `ft1-sys` compiles `libft1` via
CMake, the Rust build needs the native modem toolchain available. On
Debian/Ubuntu (or WSL2):

```sh
sudo apt install gfortran cmake ninja-build libfftw3-dev libboost-dev pkg-config
```

You need **single-precision FFTW3** (libfftw3-single, included in
`libfftw3-dev`), **Boost headers**, **CMake**, **Ninja**, and **gfortran**.
Then, from the repo root:

```sh
cargo build
cargo test --workspace   # headless: modem + engine + net + DX1 round-trips
```

These tests run headless and do not require a sound card or radio.

### UI (Vite + React + TypeScript)

```sh
npm --prefix ui install
npm --prefix ui run build   # tsc -b && vite build
```

Type-check and run the UI test suite:

```sh
cd ui && npx tsc --noEmit && npx vitest run
```

`npm --prefix ui run dev` starts the Vite dev server for UI work.

### Tauri desktop shell

The desktop app lives in `src-tauri/` (its own workspace). Building the full
app with the live radio loop pulls in `tempo-audio` and needs the platform
audio dev libraries (ALSA / CoreAudio / WASAPI) at build time. Without the
`radio` feature the shell serves app state but does not key the radio — handy
for UI-only work. You'll need `tauri-cli` v2 (`cargo install tauri-cli
--version '^2'`).

### Windows build

Windows is the primary target. Two supported paths, both documented fully in
[`WINDOWS.md`](WINDOWS.md):

- **Native (MSYS2 UCRT64)** — `scripts/build-windows.sh`, with the PowerShell
  wrapper `scripts/build-windows.ps1`. Needs the MSYS2 UCRT64 toolchain
  (`mingw-w64-ucrt-x86_64-{gcc,gcc-fortran,cmake,ninja,fftw,boost,pkgconf}`),
  the Rust GNU toolchain (`rustup default stable-x86_64-pc-windows-gnu`),
  Node.js LTS, and `tauri-cli` v2.
- **Cross-compile from Linux / WSL2** — `scripts/build-windows-cross.sh` (no
  MSYS2 needed). Needs `gcc-mingw-w64-x86-64 g++-mingw-w64-x86-64
  gfortran-mingw-w64-x86-64 cmake ninja-build nodejs npm nsis`, the
  `x86_64-pc-windows-gnu` Rust target, and `tauri-cli`. The script cross-builds
  FFTW3f for you.

Supporting scripts: `scripts/fetch-hamlib.sh` (stages `rigctld` + DLLs as a
Tauri bundle resource so the installer ships CAT control offline) and
`scripts/gen-icons.py` (app icons).

---

## Code style

- **Rust 2021.** Before opening a PR, run all three and make sure they pass:

  ```sh
  cargo fmt --all
  cargo clippy --all-targets
  cargo test --workspace
  ```

- **Keep Clippy clean.** `cargo clippy --all-targets` is currently warning-free;
  please keep it that way. If a lint genuinely doesn't apply, prefer a narrow,
  commented `#[allow(...)]` over a blanket suppression.
- **Run `cargo fmt`** — formatting is non-negotiable so diffs stay small.
- **UI:** the build runs `tsc -b`, so it must typecheck. Match the existing
  style in `ui/src`.
- Keep changes minimal and focused — touch only what the change requires.

---

## Branch / PR workflow

1. Fork (or branch off `main` if you have push access) — don't commit
   directly to `main`.
2. Make your change on a topic branch, with focused commits.
3. Run `cargo fmt --all`, `cargo clippy --all-targets`, and
   `cargo test --workspace` locally; if you touched the UI, also run
   `cd ui && npx tsc --noEmit && npx vitest run`; for a cross-compiled Windows
   build, run `./scripts/build-windows-cross.sh`.
4. Open a pull request against `main` describing **what** changed and **why**.
   For modem/waveform or protocol changes, say how you validated it (and call
   out clearly if it's simulation-only).
5. Expect review and iteration — small, well-scoped PRs merge fastest.

If you're reporting a bug rather than fixing one, an issue with reproduction
details (and, for on-air problems, band/conditions/SNR) is hugely valuable on
its own.

---

## License & Developer Certificate of Origin

Nexus is licensed under **GPL-3.0-or-later** (full text in
[`COPYING`](COPYING)). By contributing, you agree that your contributions are
licensed under **GPL-3.0-or-later**.

Nexus derives from WSJT-X (Joe Taylor K1JT and the WSJT Development Group,
GPLv3) and bundles/links other free software — see [`NOTICE`](NOTICE) for the
heritage and third-party licenses. Keep new dependencies license-compatible
with GPLv3.

Please **sign off** your commits to certify the
[Developer Certificate of Origin](https://developercertificate.org/):

```sh
git commit -s
```

This adds a `Signed-off-by: Your Name <you@example.com>` trailer, asserting you
have the right to submit the contribution under the project's license.

---

Welcome aboard, and 73. The fastest way to help Nexus cross the FT1/DX1
remaining gate is to get it on the air and tell us what you hear.
