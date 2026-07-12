# CW

The CW cockpit is a casual/ragchew CW station in software. It is deliberately
scoped: you key every message yourself — there are no contest serials and no
auto-sequencing. What it gives you is a keyboard keyer with F-key macros, a live
CW decoder so you can read the other station, on-the-fly speed control, a
zero-beat scope, and your license privileges enforced — so you can call CQ and
hold a conversation without a paddle and without copying by ear if you'd rather
not.

CW is an opt-in section. Turn it on in the first-run wizard's "which modes?"
step or in [Settings ▸ Features](settings-reference.md#features).

<!-- TODO: capture screenshot — the CW cockpit — macros, WPM control, AF scope, log strip -->

## The tour

**Keyer back-ends.** Three keyers ship and are selectable at the top of the
cockpit:

- **CAT** — the rig generates the Morse (Hamlib `send_morse`), with speed pushed
  over CAT. Zero extra hardware; needs a rig that supports CW keying over CAT.
- **Soundcard** — Nexus synthesizes PARIS-timed, click-free Morse (5 ms
  raised-cosine envelopes) through the TX audio path, for rigs without a CW keyer
  command. This works **only** if Nexus's audio output is routed to the rig
  (as for FT8) *and* PTT works — otherwise it looks like it's sending but nothing
  reaches the air. The rig goes to USB/LSB for this path.
- **WinKeyer** — a K1EL WinKeyer hardware keyer over serial (rig in CW). It's the
  no-ambiguity option: real hardware timing, nothing to route. Set its serial
  port under **WinKeyer port** in
  [Settings ▸ Rig / CAT](settings-reference.md#rig--cat).

**The CW decoder.** A live single-signal decoder reads the receive audio at your
marker pitch and prints a running transcript that persists as text scrolls by,
along with the decoded WPM (until you set WPM by hand, the keyer can follow the
decoded speed). A **sensitivity** slider trades false characters against
weak-signal copy: slide it down and spurious characters thin out on a noisy
frequency; slide it up to catch weaker or off-pitch signals and QSB. The middle
is the default. A **copilot** row shows the decoded callsigns as chips and, in
Guided mode, prompts the next logical over; switch it to Expert for just the call
chips.

**Speed.** WPM runs 5–50 (default 25). Nudge it on the fly with **PgUp / PgDn**
(±2 WPM, hold Shift for ±4).

**The AF scope** is a narrow 300–1100 Hz display with a hairline drawn at your
sidetone pitch, so you can zero-beat a station by ear and eye.

<!-- TODO: capture screenshot — the eight F-key macro buttons with the recommended-next highlight -->

## Macros

Eight F-key macros cover a normal ragchew, in the order you'd send them:

| Key | Label | Sends |
|---|---|---|
| `F1` | CQ | `CQ CQ DE {MYCALL} {MYCALL} K` |
| `F2` | Call | answer a CQ with just your call (so they copy it — no report yet) |
| `F3` | Reply | your report + name, once they've come back to you |
| `F4` | 73 | sign off (`TU 73 SK`) |
| `F5` | My Call | `{MYCALL}` |
| `F6` | His Call | the worked station's call |
| `F7` | AGN | `AGN AGN` |
| `F8` | ? | a bare `?` |

Macros expand `{MYCALL}`, `{NAME}`, `{RST}`, and his-call tokens, and **599 is
cut down to `5NN`** automatically. Set your operator name (for `{NAME}`) in
[Settings ▸ Station](settings-reference.md#station).

## Core workflows

### Call CQ and work an answer

1. Set your band and frequency. Entering the cockpit commands the rig to CW
   automatically (or USB/LSB on the Soundcard path).
2. Press **`F1`** to send CQ.
3. When someone answers, type or click their call into the his-call field, then
   run **`F3`** (report + name) → **`F4`** (73) as the QSO progresses. You send
   each over — nothing fires automatically.
4. **`Esc`** aborts keying instantly: it clears the queue and stops the rig.

### Read the other station

1. Zero-beat the station so its tone sits at your marker pitch (use the AF
   scope's hairline). The decoder reads that one pitch.
2. Watch the transcript fill in, with the decoded WPM beside it. Leave WPM on
   auto and the keyer matches the station's speed for you.
3. If a noisy frequency is throwing false characters, slide **sensitivity** down;
   if a weak or drifting signal is being dropped, slide it up.
4. Click a decoded-call chip in the copilot to make that station your worked peer
   — it fills the his-call token in your macros and the log strip.

### Land here from the Needed board

Click a CW row on the [Needed board](needed-dx.md) and Nexus QSYs to the spot and
opens this cockpit with the **callsign already typed** in the log strip, ready
for your first over. The log strip pre-fills **CW / 599**.

## Honest limits

- **The decoder is single-signal**, reading the one station at your marker pitch
  — it is not a full-band skimmer that copies everything at once. Zero-beat the
  station you want, and expect ordinary machine-copy behavior: clean sending
  decodes well, heavy QSB and swamped signals less so.
- **No contest exchanges or serials** — this is a casual keyboard station by
  design. (Contest exchange modes aren't built.)

The license-class gate blocks keying outside your segment — including the
Technician CW-only segments on 80/40/15 m. Set your class in
[Settings ▸ Station](settings-reference.md#station).

## Related guides

- [Phone (SSB)](phone.md)
- [Operate — FT8/FT4 digital](operate-digital.md)
- [Needed — DX that's on the air now](needed-dx.md)
- [Settings reference](settings-reference.md)
