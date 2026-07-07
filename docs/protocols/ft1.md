# FT1 — chat-speed weak-signal HF

*A new digital protocol by KD9TAW. This page is the approachable
story; for the full DSP and math, see [FT1-Protocol.md](../FT1-Protocol.md).*

> **Read this first.** FT1 is **experimental and in open beta.** Every sensitivity
> number below is **simulation-validated only** — bench sweeps in AWGN and
> Rayleigh fading, not on-air proven. And the headline honesty: **FT1 does not
> beat FT8 on raw sensitivity.** It trades roughly 6 dB of single-shot reach
> (−15 vs ~−21 dB, simulated) for a nearly 4× faster cycle plus a retransmission-
> combining path FT8 doesn't have. Proving
> the tradeoff pays off on real bands is exactly what this beta is for.

---

## Why a 4-second cycle changes the feel

If you have run FT8, you know the rhythm: you send, then you wait 15 seconds, then
they send, then you wait again. A three-line exchange takes a couple of minutes. It
works, but it feels like passing notes under a door.

FT1 runs a **4-second transmit/receive cycle** instead of FT8's 15 seconds (FT4 is
7.5). That one change is what the whole mode is built around. Four seconds is short
enough that a back-and-forth stops feeling like a slideshow and starts feeling like
a conversation — you type, they read, they answer, all inside the time it used to
take to send one FT8 frame. That is the point of FT1: **keyboard chat at
conversation speed, still down in the weak-signal noise.**

The cost of a short cycle is sensitivity — there is less time to integrate the
signal out of the noise. That is real physics, not a bug, and it is why FT1 is a
*companion* to a slower robust tier ([DX1](dx1.md)), not a replacement for
everything.

## Same message set as FT8 — the ecosystem carries over

FT1 does **not** invent a new way to say "CQ." It reuses the **WSJT-X 77-bit
message format** — the exact same source coding FT8, FT4, and JS8 all use — wrapped
in the same **LDPC(174,91) forward error correction** and **14-bit CRC**.

Practically, that means:

- Callsigns, grids, signal reports, and free text pack into 77 bits exactly as they
  do in FT8. Nothing about *what* you can say changes.
- The Chat, QSO, and Field-Day message forms ride identically on FT1, DX1, and the
  FT8/FT4 tiers — one message layer, several waveforms.
- Longer free-text messages are **chunked across multiple 4-second frames** and
  reassembled into the chat stream. The fast cycle is what keeps multi-frame text
  feeling responsive instead of glacial.

The waveform underneath is new; the *language* is the one the digital ecosystem
already speaks.

## IR-HARQ — a failed decode isn't wasted

This is the part of FT1 with no analog in FT8, FT4, or JS8, so it is worth
explaining plainly.

In FT8, every transmission is a self-contained block. If a frame is too weak to
decode, it is gone — the receiver throws it away and your only recourse is to send
the whole thing again and hope the next copy is strong enough on its own.

FT1 borrows a trick from the cellular world called **IR-HARQ** (incremental-
redundancy hybrid ARQ). When a frame fails to decode, the receiver **keeps it**
instead of discarding it. The retransmission then sends *new* parity bits — extra
error-correction the receiver has never seen — and the decoder **combines** the
saved frame with the new one, effectively decoding a longer, stronger code than
either transmission carried alone.

So the second attempt isn't a fresh coin flip. It builds on the first. Two weak
copies that would each fail on their own can succeed *together*. Send a third, and
it combines again. This is the cellular heritage: your phone does the same thing
when a data packet arrives corrupted — it asks for incremental redundancy, not a
naive resend.

### Redundancy versions, carried for free

Each transmission is a **redundancy version** — RV0 (the first send), then RV1, then
RV2. The clever part is how the receiver knows which one it is looking at: each RV
carries a **different Costas synchronization pattern** (the little frequency-hop
fingerprint the receiver locks onto). RV0, RV1, and RV2 each use a distinct variant,
so the receiver identifies the version *during sync*, before it even demodulates the
data. **There are zero extra bits spent signalling the RV** — it rides entirely on
structure the protocol already has.

### Backward-compatible by design

RV0 is a completely standard LDPC(174,91) frame. A station that knows nothing about
IR-HARQ just decodes RV0 like any other block code and never notices the mechanism
exists. And if a retransmission goes unacknowledged, an FT1 station falls back to
resending RV0 — so even a legacy-style receiver can average two standard copies.
Nothing about IR-HARQ breaks a simpler decoder.

In Nexus, IR-HARQ is **live and on by default.** The QSO sequencer escalates the
redundancy version automatically (RV0 → RV1 → RV2 as sends go unanswered, resetting
when the exchange completes), a decode badge shows how many versions were combined,
and there is a HARQ on/off toggle for A/B comparison.

## The honest numbers

Every figure in this table is **simulation-validated** — from the project's AWGN and
Rayleigh-fading test harnesses, re-validated in the Rust test suite and the Windows
cross-build. **None of it is on-air proven yet.** Thresholds are 50%-decode points
referenced to a 2500 Hz noise bandwidth, the standard yardstick for these modes.

| What | Figure | Basis |
|---|---|---|
| FT1 standalone threshold | **≈ −15 dB** AWGN | simulation |
| FT8, for comparison | ≈ −21 dB | commonly-cited operational figure |
| Gap FT1 gives up to FT8 | **≈ 6 dB** (and ≈ 2.5 dB vs FT4) | simulation vs. cited FT8 |
| IR-HARQ combiner gain (3-TX) | **+1.3 dB** AWGN, **+3.2 dB** under 1 Hz / 1 ms fading | simulation |
| Through the full live pipeline | **≈ +2.5 dB** threshold shift and **≈ 2×** QSO completion in the −11…−13 dB **AWGN** zone | simulation + cross-build |
| RV auto-detection | > 99% accurate, < 1% false, down to −11 dB | simulation |
| Occupied bandwidth | **not specified (est. ~42–67 Hz)** | not a published constant |

A note on that last row: FT1's occupied bandwidth is **not** published as a hard
number anywhere in the source. Estimates range roughly 42–67 Hz depending on how you
measure. Treat it as "designed to be narrow," not as a spec — and on cramped bands
(17 m, 12 m) prefer [DX1](dx1.md)'s tighter ~50 Hz signal.

## The FT8 tradeoff, stated plainly

It bears repeating because it is the single most important thing to understand about
this mode:

**FT1 is not more sensitive than FT8. It is faster.**

FT8's ~−21 dB single-shot threshold makes it the most sensitive mode here; FT1's
~−15 dB puts it roughly where FT4 sits. FT1 gives up about **6 dB** of raw reach to
FT8 (and about 2.5 dB to FT4) in exchange for a cycle that is **nearly 4× shorter**
*and* the IR-HARQ path that claws sensitivity back over several short transmissions
instead of one long one. This is the same kind of bargain FT4 already makes against
FT8 (≈3.5 dB for half the cycle) — FT1 just pushes the lever further and adds
incremental redundancy.

If you need to dig out the weakest possible signal in one shot, use FT8. If you want
a *conversation* down in the noise, that is what FT1 is for. And if the path is
fading, reach for [DX1](dx1.md), which is built to survive exactly the conditions
that hurt a coherent mode like FT1 most.

## Status, and how to help

FT1 is in **open beta.** The modem is implemented, IR-HARQ is live end-to-end and on
by default, and every number above holds up in simulation and in the Windows
cross-build self-tests. What is **not** yet done — and it is the project's #1 gate —
is **on-air characterization:** decode-rate-versus-SNR on real bands, with real
propagation, real QRM, and real operator timing that no simulation captures.

That is where you come in. The most valuable thing you can contribute is an honest
on-air report:

- band and dial frequency, and which tier (FT1 vs DX1),
- distance and rough conditions,
- what you decoded versus what you expected,
- and anything that surprised you — false decodes, RVs that combined, stations you
  saw that others didn't.

Run it, and send the decodes. That feedback is what turns these simulation numbers
into real, trustworthy operating specs.

---

**More detail:** the full waveform, FEC, sync, and IR-HARQ math — with the exact
constants from the modem source — lives in
[FT1-Protocol.md](../FT1-Protocol.md). For how to pick between the tiers, see the
[protocol overview](index.md); for the robust tier, [DX1](dx1.md).

*License: GPL-3.0 · Repository: <https://github.com/kd9taw/nexus>*
