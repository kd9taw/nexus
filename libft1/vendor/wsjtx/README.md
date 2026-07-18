# Vendored WSJT-X–derived modem source (`libft1`)

This directory is the **complete corresponding source** (GPL-3.0-only, §6) for
the FT8 / FT4 / FT1 modem that `libft1` compiles into the Nexus binary. It is
vendored in-tree so that a fresh clone builds the modem with **no external
checkout** — `libft1/CMakeLists.txt` points `WX` here by default.

## Provenance and license

The DSP sources here are derived from **WSJT-X**, © Joe Taylor (K1JT) and the
WSJT Development Group (Steve Franke K9AN, Bill Somerville G4WJS, Nico Palermo
IV3NWV, and others), licensed under the **GNU GPL, version 3** (GPL-3.0-only; the vendored `lib/` files carry no per-file license headers, so no "or later" grant applies).

- Upstream project: <https://sourceforge.net/projects/wsjt/>
- Full license text: `COPYING` at the repository root.

This is a **subset** of WSJT-X's `lib/` — only the ~70 Fortran/C/C++ DSP sources
that `libft1` actually compiles (see the source list in `libft1/CMakeLists.txt`).
None of WSJT-X's Qt/GUI code is included.

## What is original vs. reused

- **New protocol code by KD9TAW (2026)**, derived from the WSJT-X framework: the
  FT1 4-CPM turbo modem under `lib/ft1/` (`genft1`, `gen_ft1wave`,
  `turbo_decode_ft1`, `cpm_trellis`, `bcjr_cpm`, `matched_filter_bank`,
  `ft1_interleave`, `ft1_demod_bcjr`, `ft1_rv_detect`, `ft1_sync`,
  `ir_harq_combine`, `ldpc348_91`, `ft1_params`) and the FT1 acquisition decoder
  `lib/ft1_decode.f90`.
- **Reused from WSJT-X**, some files **modified by KD9TAW (2026)** to compile
  headlessly (no Qt, no shared memory, no streaming decode loop): the FT8 sources
  (`lib/ft8/`), FT4 sources (`lib/ft4/` + `lib/ft4_decode.f90`), 77-bit message
  packing (`lib/77bit/packjt77.f90`), the LDPC(174,91) / CRC / FFT infrastructure
  (`lib/ft8/*ldpc*`, `lib/chkcrc*`, `lib/crc*`, `lib/four2a.f90`,
  `lib/fftw3mod.f90`), and shared helpers at `lib/` top level.

All files in this directory — original and modified — are distributed under
**GPL-3.0-only**, the same license as WSJT-X and as Nexus as a whole. See the
project `NOTICE` for the full dependency and lineage summary.
