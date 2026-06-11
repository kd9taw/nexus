# Security Policy

Nexus is a small, volunteer, open-source project — an all-mode amateur radio
operations center (FT8/FT4 digital with WSJT-X parity, CW, SSB phone,
propagation intelligence, logging/awards, POTA/SOTA, Field Day) that also
carries the experimental Tempo FT1/DX1 chat layer. We take security seriously
within the limits of what a one-person hobby project can promise. Please read
this whole page before reporting; it sets honest expectations.

## Supported versions

| Version | Supported            |
| ------- | -------------------- |
| 0.1.x   | Yes (current series) |
| < 0.1.0 | No                   |

Nexus 0.1.x is the current series. Published Windows binaries are
cross-compiled. The FT8/FT4 tier is in daily production use; the FT1/DX1 modem
has been validated in **simulation only** — not yet on-air. There is no
separate "stable" branch; fixes land on the 0.1.x line.

## Reporting a vulnerability

**Please report security issues privately. Do not open a public GitHub issue
for a suspected vulnerability.**

Email: **kd9taw@protonmail.com** (Seth McCallister, KD9TAW)

If you would like to encrypt, say so in a first low-detail message and we can
arrange a key out of band.

Helpful things to include:

- A description of the issue and why you think it is a security problem.
- The Nexus version (e.g. `0.1.0`) and OS (Nexus's primary target is Windows).
- Steps to reproduce, a proof of concept, or a crashing input if you have one.
- The affected component (see scope below) if you can identify it.

Once a fix is released we are happy to credit you, or to keep you anonymous —
your choice. There is no bug bounty; this is an unpaid project.

## Response expectations

This is a **best-effort, volunteer** project with no SLA. Realistically:

- We aim to acknowledge a report within about **one week**.
- Triage and any fix happen as time allows; complex issues may take a while.
- We will tell you honestly if something is out of scope, won't be fixed, or
  is a known limitation rather than a bug.

If you do not hear back, a polite follow-up email is welcome.

## Scope — attack surfaces

Nexus is a **desktop** application. Its trust boundary is the local machine and
the operator's own LAN. The surfaces most relevant to a security report:

- **WSJT-X-compatible UDP listener** — Nexus speaks the WSJT-X UDP protocol
  (magic `0xADBCCBDA`, schema 3). The conventional sink is `127.0.0.1:2237`.
  Besides outbound Status/Decode datagrams, Nexus's server can accept inbound
  control datagrams: **Reply** (type 4), **HaltTx** (type 9), and **FreeText**
  (type 10) — i.e. an unauthenticated UDP peer that can reach this socket may
  be able to influence what the station transmits. **Keep this bound to
  localhost or a trusted LAN; do not expose it to untrusted networks or the
  public internet.** The protocol has no authentication (it never did in
  WSJT-X either).

- **PSK Reporter outbound** — Nexus can send spot reports as outbound UDP to
  `report.pskreporter.info:4739`. This is opt-in reporting traffic and leaves
  your machine; treat it as you would any telemetry. It carries the callsign,
  grid, and reception data you would expect a spotting upload to contain.

- **`rigctld` control port (localhost TCP)** — for CAT/PTT control Nexus
  launches Hamlib's `rigctld` itself and connects to it over TCP on localhost
  (default port **4532**). Anything that can reach that local TCP port can key
  your radio and change frequency/mode. It is intended to be localhost-only.

- **Nexus spawns `rigctld`** — when CAT PTT is selected, Nexus runs the bundled
  `rigctld` as a child process with the configured rig model, serial port, and
  baud rate. Issues in how that process is launched (argument handling, the
  bundled binary, the chosen port) are in scope.

- **Credential storage** — all service credentials (LoTW, QRZ, ClubLog, eQSL,
  and any future connectors) are stored exclusively in the **OS keychain**
  (Windows Credential Manager, macOS Keychain, or the platform secret service
  on Linux). They are never written to config files or logs.

- **Settings file** — operator/station settings are persisted as plain JSON.
  On Windows this is `%APPDATA%\tempo\settings.json` (on Unix,
  `$XDG_CONFIG_HOME/nexus/settings.json` or `~/.config/nexus/settings.json`).
  It is not encrypted and is readable by the local user account; do not treat
  it as a secret store. Credentials are not present here — see above.

### Likely out of scope

- Attacks requiring an already-compromised local machine or local admin.
- The inherent properties of amateur radio: HF is an open, unauthenticated,
  unencrypted medium by law and by design. Nexus does not (and must not, under
  Part 97) encrypt over-the-air traffic. Spoofed or replayed RF frames are a
  property of the band, not a Nexus vulnerability.
- The WSJT-X UDP protocol and PSK Reporter format themselves (we implement the
  existing, unauthenticated protocols for compatibility).
- Vulnerabilities in third-party dependencies (Hamlib, FFTW, Tauri, WebView2,
  etc.) — please report those upstream; we will pick up fixed versions.
- Theoretical issues with no realistic exploit path on a single-user desktop.

When in doubt, email us anyway — a quick "is this in scope?" is fine.

## Disclosure

We prefer **coordinated disclosure**: report privately, give us a reasonable
chance to ship a fix on the 0.1.x line, then disclose publicly. Given the
volunteer pace, please be flexible on timing and talk to us before going
public. We will not take legal action against good-faith research that
respects this policy and avoids harm to others' stations or data.

## No warranty

Nexus is licensed under **GPL-3.0-or-later** (see `COPYING`). As stated there,
the software is provided **"as is", without warranty of any kind**, to the
extent permitted by law. This security policy is a statement of good-faith
intent from a volunteer project, not a contractual guarantee.

---

*Nexus — <https://github.com/kd9taw/nexus> — Seth McCallister (KD9TAW)
&lt;kd9taw@protonmail.com&gt;*
