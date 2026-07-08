# Logbook & QSL

The logbook is Nexus's system of record: a persistent ADIF 3.1.4 store with full
round-trip fidelity. It holds callsign, grid, entity, state, band, frequency,
mode, the string RSTs each mode actually uses (FT8's "−12" and CW's "599" are
both first-class), name/QTH, notes, POTA/SOTA references, and a per-QSO upload
state for every online service — so "what has actually been uploaded where"
survives restarts.

![TODO screenshot: the logbook table with the QSL column and confirmation marks](img/TODO-logbook-table.png)

## The tour

Each row is one QSO. The columns you'll use most:

- **Call, band, mode, date/time, RST** — the contact itself.
- **Entity / DXCC** — resolved from cty.dat, first-class in the table.
- **QSL** — *which source* confirmed the contact:
  - **L** — LoTW,
  - **C** — a paper card,
  - **E** — eQSL.

  Hover for the eligibility tooltip. **eQSL is clearly labelled non-award**: an
  E confirmation counts as a confirmation but not toward DXCC/WAS (ARRL doesn't
  accept eQSL). This matters — see [Awards & Journey](awards-journey.md).

**Search** matches callsigns *and grids*. The **"needs confirmation"** chip
beside the search box filters to contacts without an award-eligible (LoTW/paper)
confirmation — a QSL you've *requested* but not received still counts as
unconfirmed and stays in that list.

![TODO screenshot: the QSL column tooltip explaining L / C / E eligibility](img/TODO-logbook-qsl-tooltip.png)

## Core workflows

### Add or edit a QSO by hand

Manual entry seeds the draft from **what you were actually running**: log a
contact from the [Phone cockpit](phone.md) and the draft says SSB, from
[CW](cw.md) it says CW — no more accidental "FT8" voice contacts. Edit any field
inline; the store round-trips to ADIF, so an export re-imports without loss.

### Upload to LoTW

1. Set your **LoTW Station Location** (and optionally the TQSL path) in
   [Settings ▸ Confirmations](settings-reference.md#confirmations). Nexus signs
   through *your installed TQSL* against that named Station Location — no
   certificate or password is stored by Nexus.
2. Click **Upload to LoTW** in the logbook. The button shows the count of
   un-uploaded QSOs; it signs and uploads the unsent batch.
3. Pull confirmations back with **Sync LoTW now** (Settings ▸ Confirmations). The
   first sync pulls your whole history; later syncs are incremental. Syncing also
   marks which of *your* uploads LoTW holds on file, so a pending contact reads
   "waiting on the other op," not "never uploaded."

### Push a single QSO to QRZ / ClubLog / HRDLog

Auto-upload (configured per service in
[Settings ▸ Confirmations](settings-reference.md#confirmations)) pushes each QSO
as you log it. When one fails — a service was down, a key was wrong — the logbook
gives you a **per-row re-push** for **QRZ**, **ClubLog**, and **HRDLog.net** so
you can retry that one contact after fixing the cause. A "duplicate" result is
the benign "already there" answer, not an error.

### Mark a QSL sent

When you send a card or request, record it on the contact with **Mark QSL sent**,
choosing the method — **bureau**, **direct**, or **electronic**. The row then
shows a quiet "QSL sent … via …" note. Marking a request sent does **not**
confirm the contact — it stays in the "needs confirmation" list until the reply
comes back.

### Understand why a contact isn't confirmed

A per-QSO **diagnostics** view explains why award credit hasn't landed yet — no
upload sent, waiting on the partner, a date mismatch — with one-click fixes where
they exist. Reconciliation tolerates ±1 day of midnight skew and matches by
mode-class, so an FT4-vs-FT8 labelling difference doesn't orphan a confirmation.

## How uploads flow

Uploads happen in the **backend log funnel**: when a QSO is logged, the
configured connectors push it. You don't push from the logbook UI for the
auto-upload path — the per-row buttons are for *recovery* when an automatic push
failed. Credentials live only in the **OS keychain**; the Connections panel reads
back presence ("credential stored"), never the secret itself.

## Honest limits

- **eQSL never counts toward LoTW-grade awards** — it's a separate confirmation
  tier, enforced everywhere credit is computed.
- **HRDLog.net is a logging/awards site, not an ARRL confirmation source** — an
  upload there never earns DXCC/WAS credit.
- **Nexus doesn't store your TQSL certificate or LoTW signing password** — LoTW
  signing is delegated to your installed TQSL.

## Related guides

- [Awards & Journey](awards-journey.md)
- [Settings reference — Confirmations](settings-reference.md#confirmations)
- [Operate — FT8/FT4 digital](operate-digital.md)
- [Contesting & POTA/SOTA](contesting-pota.md)
