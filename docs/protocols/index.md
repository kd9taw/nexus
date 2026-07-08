# Protocols in Nexus — which mode, when

Nexus operates four digital tiers over one shared message layer. Two are the proven
WSJT-X standards you already know; two are new, experimental protocols in open beta.
This page is the picker. The short version:

- **FT8 / FT4** — the proven, interoperable workhorses. Use these for everyday DX and
  contesting. Nothing experimental here.
- **[FT1](ft1.md)** — new. A 4-second cycle for *fast keyboard chat* down in the
  noise. Open beta.
- **[DX1](dx1.md)** — new. A robust, fading-resilient tier for rough paths. Open
  beta.

Every tier carries the **same 77-bit WSJT-X message payload** and the same
**LDPC(174,91) + CRC-14** error correction — the same callsigns, grids, reports, and
free text. Only the waveform and the T/R cycle change underneath. Picking a tier is a
speed-versus-reach-versus-robustness decision, not a decision about what you can say.

## One honest table

Performance figures for **FT1 and DX1 are simulation-validated only** — not yet
on-air proven. FT8/FT4 figures are the commonly-cited operational values. Thresholds
are 50%-decode points in a 2500 Hz reference bandwidth.

| | **FT8** | **FT4** | **[FT1](ft1.md)** | **[DX1](dx1.md)** |
|---|---|---|---|---|
| Status | proven standard | proven standard | **open beta** | **open beta** |
| Best for | max sensitivity, everyday DX | fast contesting | fast keyboard chat | fading / rough paths |
| T/R cycle | 15 s | 7.5 s | **4 s** | 15 s |
| Modulation | 8-GFSK | 4-GFSK | 4-CPM (coherent) | 8-FSK (non-coherent) |
| Occupied BW | ~50 Hz | ~90 Hz | not specified (est. ~42–67 Hz) | 50 Hz |
| Retransmission combining | no | no | **yes — IR-HARQ** | no |
| AWGN threshold | ~−21 dB | ~−17.5 dB | ~−15 dB *(sim)* | ~−18.6 dB *(sim)* |
| Fading behavior | coherent-class (10+ dB hit) | — | fragile (coherent) | **~3.7 dB penalty** *(sim)* |
| Interop | WSJT-X ecosystem | WSJT-X ecosystem | Nexus (new waveform) | Nexus (new waveform) |

## How to choose

- **Want maximum sensitivity, or to work the wider FT8 world?** Use **FT8.** It is
  the most sensitive single-shot mode here and everyone runs it.
- **Contesting, want to move fast but keep the ecosystem?** Use **FT4.**
- **Want an actual conversation — ragchew, keyboard-to-keyboard — while still weak?**
  Use **[FT1](ft1.md).** The 4-second cycle is what makes it feel like talking, and
  IR-HARQ lets weak retransmissions combine instead of being wasted. Just remember:
  **FT1 is not more sensitive than FT8 — it is faster.** It trades ~6 dB for that
  speed.
- **Path fading, fluttering, or long and marginal?** Use **[DX1](dx1.md).** It gives
  up a little raw reach to be nearly immune to the fading that collapses coherent
  modes.

## The honesty banner

FT1 and DX1 are **experimental modes in open beta.** The waveforms are implemented
and FT1's IR-HARQ is live, but every sensitivity number is from **simulation** — AWGN
and Rayleigh-fading bench sweeps, re-validated in the test suite and the Windows
cross-build. They are **not on-air proven.** Decode-rate-versus-SNR on real bands is
the project's #1 remaining gate, and honest field reports are the most valuable thing
a beta operator can send back.

Two practical notes:

- FT1 and DX1 use **new waveforms**, so they transmit on Nexus's own **calling
  frequencies**, deliberately clear of the FT8/FT4/JS8/WSPR watering holes — see
  [FREQUENCIES.md](../FREQUENCIES.md). These are proposed, editable defaults, not
  regulatory channels.
- The license-class transmit lockout applies to every tier: Nexus refuses to
  transmit outside your privileges — a software guard in every TX path.

---

**Go deeper:** [FT1 explained](ft1.md) · [DX1 explained](dx1.md) · the full DSP and
math in [FT1-Protocol.md](../FT1-Protocol.md) · running alongside WSJT-X and your
logger in [interop.md](../interop.md).

*License: GPL-3.0 · Repository: <https://github.com/kd9taw/nexus>*
