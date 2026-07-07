# DX1 — the robust tier

*The fading-resilient companion to [FT1](ft1.md), by Seth McCallister, KD9TAW.
This page is the approachable story; the full DSP is in
[FT1-Protocol.md](../FT1-Protocol.md).*

> **Read this first.** DX1 is **experimental and in open beta.** Its sensitivity and
> fading figures are **simulation-validated only** — bench sweeps, not on-air proven.
> The use cases below (NVIS, polar, rough conditions) are stated as **design
> intent**, not as demonstrated results. On-air characterization is the goal of the
> beta.

---

## When fading beats sensitivity

There are two different ways for a weak-signal contact to fail, and they call for
opposite solutions.

One is simple **weakness** — the signal is just barely above the noise. The fix is
sensitivity: integrate longer, track the carrier's phase, extract every last bit of
information. That is what a *coherent* mode like [FT1](ft1.md) (or, more sensitively,
FT8) does well.

The other is **fading** — the signal is strong enough on average, but multipath and
ionospheric motion smear its phase, so the very coherence that makes a sensitive mode
sensitive gets destroyed. On a badly fading path, a coherent mode doesn't lose a
decibel or two; it can lose **10 dB or more** and simply stop decoding.

DX1 exists for the second case. It gives up some raw sensitivity to be almost
*immune* to fading, so that on the paths where FT1 and FT8 fall apart, DX1 keeps
getting through.

## How it stays robust: non-coherent 8-FSK

DX1's robustness comes from one deliberate choice: it **never tracks carrier phase**.

- It transmits **8-FSK** — eight distinct tones, each symbol carrying 3 bits, Gray
  coded so a mistaken neighbor tone costs only one bit.
- The receiver decides which tone was sent purely by **energy** — a per-symbol FFT,
  one bin per tone — and hands soft confidence values to the same LDPC(174,91)
  decoder the other tiers use. **No phase is tracked anywhere in the chain.**

Because the decision is "which tone had the most energy," not "what is the carrier's
phase doing," the fading that wrecks a coherent mode barely touches DX1. That is the
whole reason the mode exists.

## The facts

All performance figures are **simulation-validated only.** Thresholds are 50%-decode
points in a 2500 Hz reference bandwidth.

| Property | Value |
|---|---|
| Modulation | non-coherent **8-FSK**, 3 bits/symbol, Gray coded |
| Baud / tone spacing | 6.25 Hz (tone spacing = baud) |
| Occupied bandwidth | **50 Hz** (8 × 6.25 Hz) |
| Sampling | 12 kHz, 1920 samples/symbol (a 1920-point FFT gives exactly 6.25 Hz/bin) |
| Sync | linear **chirp preamble** (~0.64 s, swept across the 50 Hz band) |
| Frame | chirp + 58 data symbols = 9.92 s of transmit inside a **15-second** T/R slot |
| Payload / FEC | same **77-bit** message set + **LDPC(174,91)** + CRC-14 as FT1 and FT8 |
| AWGN threshold | **≈ −18.6 dB** *(simulation)* |
| Fading penalty | **≈ 3.7 dB** under per-symbol Rayleigh fading *(simulation)* — where coherent modes lose 10+ dB |
| Default calling carrier | 1500 Hz audio |
| IR-HARQ | **none, by design** (see below) |

That **3.7 dB fading penalty is the entire point of the mode.** A coherent mode on
the same fading channel loses several times that. DX1 trades a few dB of best-case
sensitivity for a signal that barely notices the fading in the first place.

## Full-passband acquisition

Like FT1's decoder, DX1 does **not** listen on just one frequency. Every slot it
scans the **entire audio passband (200–2900 Hz)** and decodes every DX1 signal it
finds, in three stages:

1. a coarse chirp-correlation sweep of candidate carriers on a 12.5 Hz grid,
2. a median-threshold peak-pick to find the real signals,
3. a full CRC-14-gated decode of each survivor, so false peaks are rejected.

The scan takes roughly 3–4 seconds per slot and returns up to 16 decodes. Your chosen
RX frequency becomes a waterfall marker and a TX-pairing hint rather than the one
frequency you can hear on.

## No HARQ — and why that's the right call

FT1's signature IR-HARQ retransmission-combining is **deliberately absent** from DX1.
The two features solve different problems and don't mix well:

- IR-HARQ pays off by accumulating redundancy across several *short* transmissions —
  it is a natural fit for FT1's 4-second cycle.
- DX1's robustness already comes from *within* each 15-second frame, on paths where
  the phase-sensitive machinery IR-HARQ leans on is exactly what's unreliable.

So DX1 keeps it simple: one strong, fading-proof frame per slot, decoded on its own.
A DX1 decode reports no redundancy version because there is none.

## What DX1 is designed for

These are the conditions the mode was **designed** to handle. They are design intent,
not proven claims — confirming them on the air is part of the beta:

- **NVIS and regional nets** — near-vertical-incidence skywave, where signals are
  strong but the near-vertical path fades and flutters.
- **Polar and high-latitude paths** — auroral flutter and rapid Doppler spreading
  that shred carrier phase.
- **Rough, disturbed conditions generally** — the marginal-DX openings and unsettled
  bands where a coherent mode's decode rate collapses but a robust one hangs on.

If the path is stable and you want speed, use [FT1](ft1.md). If the path is fading,
flaky, or long-and-marginal and you are willing to slow to a 15-second cycle for
reliability, that is DX1. In Nexus the choice is a visible per-transmission toggle —
both tiers carry the identical message, just at different speed-versus-robustness
points.

---

**More detail:** the DX1 waveform, chirp sync, and full-passband scan — with exact
constants — are documented in [FT1-Protocol.md §7.2](../FT1-Protocol.md). To choose
between the tiers, see the [protocol overview](index.md).

*License: GPL-3.0 · Repository: <https://github.com/kd9taw/nexus>*
