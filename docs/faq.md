# Nexus FAQ

Answers for new operators and for hams evaluating Nexus against the tools they
already run. Nexus is in **open beta**: the FT8/FT4 core is production-grade and
verified against WSJT-X behavior; the newest features are fresh from the bench and
field reports are wanted.

---

## Operating and modes

### Is this a WSJT-X replacement?

For **FT8 and FT4 operating**, that is the goal. The sequencer is built to WSJT-X's
behavior and verified against a 207-row parity matrix, and Nexus adds things stock
WSJT-X doesn't have — country and worked-before flags on every decode, one-click
"work it" that jumps band/mode/frequency together, and a Needed board that ranks the
stations on the air by what they are worth to *your* log.

But WSJT-X still has **more modes** (Q65, MSK144, WSPR, and others Nexus does not
implement), and it runs on macOS and Linux where Nexus ships Windows only today. And
Nexus speaks WSJT-X's UDP protocol, so you don't have to choose all-or-nothing — your
GridTracker/JTAlert/logger workflow survives either way. See [interop.md](interop.md).

### What is FT1, in one sentence?

FT8's message set on a 4-second cycle with cellular-style retransmission combining —
keyboard chat at conversation speed, still down in the weak-signal noise (open beta;
simulation-validated). The longer story is in [protocols/ft1.md](protocols/ft1.md).

### Is FT1 more sensitive than FT8?

**No — and this is the most important thing to understand about it.** FT1 trades
roughly 6 dB of raw single-shot sensitivity against FT8 (about 2.5 dB against FT4)
for a nearly 4× faster cycle plus an IR-HARQ path that lets weak retransmissions
combine instead of being wasted. FT8's ~−21 dB threshold is the most sensitive here;
FT1's ~−15 dB sits about where FT4 does. Those numbers are **simulation-validated
only.** If you want maximum reach in
one shot, use FT8; if you want a conversation, use FT1. When the path is fading, use
[DX1](protocols/dx1.md).

### What is DX1?

The robust tier: non-coherent 8-FSK on a 15-second cycle, built to shrug off the
fading that collapses coherent modes (a ~3.7 dB fading penalty in simulation, where
coherent modes lose 10+ dB). It gives up some raw sensitivity to stay decodable on
NVIS, polar, and rough paths. Details in [protocols/dx1.md](protocols/dx1.md).

### Are the FT1/DX1 performance numbers proven?

They are **simulation-validated** — AWGN and Rayleigh-fading bench sweeps, re-checked
in the test suite and the Windows cross-build. They are **not on-air proven.**
Decode-rate-versus-SNR on real bands is the project's #1 remaining gate, and it is
what the open beta exists to establish. Every dB figure Nexus publishes is labeled
"simulated" for exactly this reason.

### How do I help the beta?

Send honest on-air reports: band, dial frequency, which tier (FT1 or DX1), distance
and rough conditions, and what you decoded versus what you expected — including the
surprises (false decodes, retransmissions that combined, stations you saw that others
didn't). Field reports are the single most useful contribution right now. Use the
issue tracker at <https://github.com/kd9taw/nexus>.

---

## Setup and safety

### Do I need to install Hamlib, WebView2, or drivers first?

No. Hamlib (for CAT and rotator control) and the WebView2 runtime are **bundled** in
the installer, so Nexus works on a bare PC with no separate installs. If Windows is
missing a USB bridge-chip driver for your rig's interface, the first-run wizard
detects that and gives you the right download link.

### Will Nexus transmit on its own?

**Never on launch.** Nexus starts passive — it listens. Every transmission is an
explicit operator action (you send a message, answer, or call CQ). On top of that
there is a transmit watchdog and a **license-class lockout** (Technician / General /
Extra per Part 97, including the 2026 60 m rules) that blocks every transmit path in
software outside your privileges. The presence beacon is off by default.

### About 50 rigs are "supported" — is mine one?

Around 50 rigs are curated out of the box (Icom including the IC-9700 and IC-705;
Yaesu FTDX10, FT-991A, FT-710; Kenwood; Elecraft; FlexRadio; Xiegu; QRP Labs QMX and
more), and Hamlib is bundled, so CAT and rotator control work offline. "Detect my
radio" scans USB and finds FlexRadios on the LAN, then fills in the model, port, and
paired audio in one click.

**Field-verified so far:** the FTDX10 and FT-991A (on real hardware, by the author).
Other rigs use Hamlib's well-established support but have not each been bench-verified
in Nexus specifically — this is a beta, and confirming your particular rig is useful
feedback.

### What's the story with FlexRadio and Xiegu?

Both work today over **Hamlib CAT** — Flex via SmartSDR's network CAT, Xiegu over
serial — and Flex is discoverable on the LAN with one-click DAX audio pairing. Honest
status:

- **FlexRadio:** the FLEX-6400M CAT path is in **final verification** on hardware. The
  deeper native SmartSDR integration (slices, panadapter, DAX as first-class objects)
  is deferred to a later phase — today Flex is driven as a network-CAT rig, not
  through the native SmartSDR API.
- **Xiegu:** supported via Hamlib CAT (e.g. the G90), but **not yet verified on
  hardware** in Nexus. If you run one, a field report is welcome.

### Why is the download about 200 MB?

Because everything is bundled: the WebView2 runtime, Hamlib (CAT + rotator), and the
whole DSP stack. That is the tradeoff for working offline on a bare PC with no
separate installs and no admin rights — a per-user install that just runs.

### Why does the installer warn that it's unsigned?

Nexus ships **unsigned** today, so Windows SmartScreen will show a warning when you
run the installer. This is expected for a beta from an individual developer — code-
signing certificates are a paid, identity-verified process. To install safely,
**verify the SHA-256 hash** of the download against the value published on the release
page before running it. Click "More info → Run anyway" once the hash matches.

---

## Data, licensing, and platforms

### Where does my data go?

Your log stays **local**, in an ADIF file on your machine. Uploads happen **only** to
the services you explicitly configure — LoTW, QRZ, ClubLog, eQSL, HRDLog.net — and
those credentials live in the **Windows keychain**, never in a plaintext config file.
Journey/achievement progress never leaves your computer. Nexus has no telemetry or
analytics phone-home; the only outbound traffic is the connectors you turn on and, by
default, PSK Reporter spot uploads (which you can disable).

### Is Nexus really free? What's the license?

Yes — Nexus is **free and open source under the GPL-3.0.** There is no paid tier, no
subscription, and no "pro" upsell. The GPL means you get the complete source, you can
study and modify it, and any distributed derivative must also be GPL-3.0. Nexus builds
on WSJT-X's GPL heritage (the 77-bit message packing, LDPC FEC, and FFTW
infrastructure), which is why the shared message layer is compatible with the modes
you already run.

### Where is the source code, and can I contribute?

The repository is <https://github.com/kd9taw/nexus>. Contributions are welcome —
issues, field reports, and pull requests all help. The most valuable contributions
during beta are **on-air decode reports** for FT1 and DX1 (see "How do I help the
beta?" above) and **rig confirmations** for radios beyond the two the author has
bench-verified. Bugs, propagation-model feedback, and interop reports against your
particular logger or cluster are all useful.

### Mac or Linux?

Not yet as a shipping build. The codebase is cross-platform Rust/Tauri, but only the
**Windows** installer ships today (built, in fact, by cross-compiling from Linux). If
you want a native macOS or Linux build, say so on the issue tracker — interest is what
prioritizes it.

### Will there be automatic updates?

Not in this beta — there is no auto-updater yet. Watch the release page (and verify
the SHA-256 of each new download) to update manually.

---

**More:** [protocol overview](protocols/index.md) · [FT1](protocols/ft1.md) ·
[DX1](protocols/dx1.md) · [interop and companion setup](interop.md)

*License: GPL-3.0 · by Seth McCallister, KD9TAW · Repository:
<https://github.com/kd9taw/nexus>*
