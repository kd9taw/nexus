# Privacy & Coordinated QSY ("move together")

> **Read this first — the honest ceiling.** On the amateur bands there is **no such thing as a private conversation.** Nexus's Coordinated QSY makes a QSO a little *harder to find and follow* for a casual listener. It is **not** encryption, **not** a secret hopping pattern, and **not** privacy. A capable listener with a wide receiver can still find you, decode every word, and follow you. Nexus is designed this way **on purpose**, to stay legal.

This is research + engineering, **not legal advice**. Your license is on the line — the authoritative rules are the FCC's (US Part 97) / your national regulator's, and the [ARRL](https://www.arrl.org/part-97-amateur-radio) is a good plain-English guide.

---

## Why real privacy is impossible (and illegal) on ham

Amateur radio is, by treaty and rule, an **open** service. The relevant US rules:

- **§97.113(a)(4)** prohibits *"messages encoded for the purpose of obscuring their meaning."* That bans **encryption and any secret code** — full stop. The mechanism that would give you real privacy (a secret key, a hidden hop sequence) is exactly what this rule targets.
- **§97.119** requires you to **transmit your callsign in the clear at least every 10 minutes.** Your identity is never hidden.
- **§97.309** requires data codes to be **publicly documented.** Nexus's waveforms (the FT1/DX1 protocols) and this QSY scheme are open-source (GPLv3) — that's the documentation.
- **§97.311** allows genuine spread-spectrum / frequency hopping **only at 222 MHz and above**, and even then it must be publicly documented and must not obscure meaning. On HF / 6 m / 2 m, only a **plain, announced QSY** is legal.

So the only lever Nexus can legally pull is **low-probability-of-intercept (LPI)** — making the QSO *harder to stumble onto*, not *harder to read*. The realistic benefit is against **casual** listeners: someone spinning the dial, a club member parked on the calling frequency, an HT scanner. Against a software-defined radio recording the whole band, **none of these techniques do anything** — and Nexus says so plainly in the app.

---

## What Coordinated QSY actually does

Two stations already in contact agree to **step to a different channel together**, and keep stepping on a schedule, with each move **announced as plain text** in the over. It is a separate, **opt-in** feature (the **Roam** tab) that is **off by default** and does not change how Chat, QSO, or Field Day behave.

- **In the clear.** The move is announced with a human-readable directive — e.g. `QSY 40M …` — carried as a normal open broadcast in the message stream. Anyone monitoring sees it.
- **Synced by the clock.** Both stations share Nexus's UTC slot clock, so the directive names an absolute slot to move on and both retune on the **same** T/R boundary — they land together.
- **Auto-negotiated, with manual override.** Of the two callsigns, the lexicographically-smaller one is the **initiator**: it announces moves on a cadence you set. The other **follows** automatically. Either operator can **Move now**, **Pause** (hold the current channel), or **Stop → home** at any time.
- **Self-healing.** If a station stops hearing its partner for several overs (a missed/QRM'd move), **both fall back to the home channel** and re-rendezvous, instead of drifting apart.

What it buys you: a real **anti-QRM** tool (walk away from interference together) and **modest obscurity** against a casual listener who isn't already parked on your current frequency. What it does **not** buy you: secrecy.

---

## Using it (the Roam tab)

1. **Get into a contact.** Select the station you're working in the roster (that peer becomes your **roaming partner**). Coordinated QSY operates in the conversational (Chat) flow — it never interrupts the auto-QSO or Field-Day sequencers.
2. **Open the Roam tab** and read the disclaimer (it's permanent — by design).
3. **Pick your channel set** — at least two band-plan channels to rotate through. Announced QSY is legal on every band.
4. **Pick a cadence** — how many overs between hops (conservative by default, never per-over, so it reads as an ordinary QSY).
5. **Enable.** The initiator starts announcing; the follower auto-follows. The status line shows the role, the current channel, and the next scheduled move.
6. **Override anytime** — Move now / Pause / Stop → home.

Both operators must have the feature enabled for a move to happen. If your partner doesn't run Nexus (or has it off), they'll simply see the directive as a plain-text message and stay put — so nothing moves unless you've both opted in.

---

## What Nexus deliberately does **not** do

- **No encryption, ever.**
- **No secret or keyed hopping pattern.** A hidden seed would be the illegal "obscuring" mechanism, and unauthorized spread-spectrum below 222 MHz besides.
- **No rapid covert hopping.** Moves are announced and paced like a normal QSY, not a spreading sequence.
- **No anonymity.** Your callsign IDs in the clear, as the rules require.

A future, clearly-labelled **opt-in** option could add a *publicly documented* hop (legal only ≥ 222 MHz, with a public seed) — but even that only inconveniences casual listeners, so it is **not** built today.

---

## TL;DR

Coordinated QSY is a legal, in-the-clear way to **move off interference and shake a casual listener** by hopping channels **together** with the station you're working. It is an operational convenience and a modest obscurity aid. **It is not private, not secure, and a capable listener can follow.** Operate within your license privileges.

---

*See also: [Frequency Plan](Frequency-Plan.md) · [FAQ](FAQ.md)*
