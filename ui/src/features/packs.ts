// Curated starter packs — ready-made channel sets an operator can install into
// their Memories with one click (or offered at first run). Bundled (works offline,
// no external dependency); a hosted refresh can layer on later. North America first.
//
// The data here is frequency *conventions* (calling channels, digital watering
// holes, POTA activity) plus a few well-known HF nets. Net schedules are UTC and
// approximate — reminders are opt-in and the operator can adjust the time.

import {
  addGroup,
  addMemory,
  coerceMemory,
  memoryKey,
  updateMemory,
  type MemoriesBank,
  type Memory,
} from './memories'

export interface PackMemory extends Partial<Memory> {
  name: string
  rxMhz: number
  mode: string
}

export interface Pack {
  id: string
  name: string
  /** One-line description shown on the pack card. */
  description: string
  /** Region tag, e.g. "North America". */
  region: string
  memories: PackMemory[]
}

// --- North America starter packs -------------------------------------------

const CALLING: Pack = {
  id: 'na-calling',
  name: 'VHF/UHF Calling & Simplex',
  description: 'National FM & SSB calling channels — the frequencies to find (and make) contacts.',
  region: 'North America',
  memories: [
    { name: '2 m FM Calling', rxMhz: 146.52, mode: 'FM', kind: 'simplex', notes: '2 m FM national simplex calling' },
    { name: '70 cm FM Calling', rxMhz: 446.0, mode: 'FM', kind: 'simplex', notes: '70 cm FM simplex calling' },
    { name: '6 m FM Calling', rxMhz: 52.525, mode: 'FM', kind: 'simplex', notes: '6 m FM simplex calling' },
    { name: '1.25 m FM Calling', rxMhz: 223.5, mode: 'FM', kind: 'simplex', notes: '1.25 m FM simplex calling' },
    { name: '23 cm FM Calling', rxMhz: 1294.5, mode: 'FM', kind: 'simplex', notes: '23 cm FM simplex calling' },
    { name: '6 m SSB Calling', rxMhz: 50.125, mode: 'USB', kind: 'calling', notes: '6 m SSB calling frequency' },
    { name: '2 m SSB Calling', rxMhz: 144.2, mode: 'USB', kind: 'calling', notes: '2 m SSB (horizontal) calling' },
    { name: '70 cm SSB Calling', rxMhz: 432.1, mode: 'USB', kind: 'calling', notes: '70 cm SSB calling' },
    { name: '10 m FM Calling', rxMhz: 29.6, mode: 'FM', kind: 'simplex', notes: '10 m FM simplex calling' },
  ],
}

const DIGITAL: Pack = {
  id: 'na-digital',
  name: 'HF Digital Watering Holes',
  description: 'Standard FT8 & FT4 frequencies across the HF bands (WSJT-X defaults).',
  region: 'Worldwide',
  memories: [
    { name: 'FT8 40 m', rxMhz: 7.074, mode: 'FT8', kind: 'digital' },
    { name: 'FT8 30 m', rxMhz: 10.136, mode: 'FT8', kind: 'digital' },
    { name: 'FT8 20 m', rxMhz: 14.074, mode: 'FT8', kind: 'digital' },
    { name: 'FT8 17 m', rxMhz: 18.1, mode: 'FT8', kind: 'digital' },
    { name: 'FT8 15 m', rxMhz: 21.074, mode: 'FT8', kind: 'digital' },
    { name: 'FT8 10 m', rxMhz: 28.074, mode: 'FT8', kind: 'digital' },
    { name: 'FT8 6 m', rxMhz: 50.313, mode: 'FT8', kind: 'digital' },
    { name: 'FT4 40 m', rxMhz: 7.0475, mode: 'FT4', kind: 'digital' },
    { name: 'FT4 20 m', rxMhz: 14.08, mode: 'FT4', kind: 'digital' },
    { name: 'FT4 15 m', rxMhz: 21.14, mode: 'FT4', kind: 'digital' },
  ],
}

const POTA: Pack = {
  id: 'na-pota',
  name: 'POTA Activity',
  description: 'Common Parks on the Air hunting frequencies — CW, SSB, and FT8.',
  region: 'North America',
  memories: [
    { name: 'POTA 40 m FT8', rxMhz: 7.074, mode: 'FT8', kind: 'pota', notes: 'POTA activity' },
    { name: 'POTA 20 m FT8', rxMhz: 14.074, mode: 'FT8', kind: 'pota', notes: 'POTA activity' },
    { name: 'POTA 40 m CW', rxMhz: 7.032, mode: 'CW', kind: 'pota', notes: 'Common POTA CW activity' },
    { name: 'POTA 30 m CW', rxMhz: 10.112, mode: 'CW', kind: 'pota', notes: 'Common POTA CW activity' },
    { name: 'POTA 20 m CW', rxMhz: 14.032, mode: 'CW', kind: 'pota', notes: 'Common POTA CW activity' },
    { name: 'POTA 15 m CW', rxMhz: 21.032, mode: 'CW', kind: 'pota', notes: 'Common POTA CW activity' },
    { name: 'POTA 80 m SSB', rxMhz: 3.985, mode: 'LSB', kind: 'pota', notes: 'Common POTA SSB activity — adjust to conditions' },
    { name: 'POTA 40 m SSB', rxMhz: 7.185, mode: 'LSB', kind: 'pota', notes: 'Common POTA SSB activity — adjust to conditions' },
    { name: 'POTA 20 m SSB', rxMhz: 14.285, mode: 'USB', kind: 'pota', notes: 'Common POTA SSB activity — adjust to conditions' },
    { name: 'POTA 15 m SSB', rxMhz: 21.285, mode: 'USB', kind: 'pota', notes: 'Common POTA SSB activity — adjust to conditions' },
    { name: 'POTA 10 m SSB', rxMhz: 28.485, mode: 'USB', kind: 'pota', notes: 'Common POTA SSB activity — adjust to conditions' },
  ],
}

const NETS: Pack = {
  id: 'na-nets',
  name: 'Well-Known HF Nets',
  description: 'A few widely-heard nets. Times are UTC and approximate — verify and enable reminders per net.',
  region: 'North America',
  memories: [
    {
      name: 'Maritime Mobile Service Net',
      rxMhz: 14.3,
      mode: 'USB',
      kind: 'hfnet',
      notes: 'Safety & health-and-welfare traffic; runs daily into the evening (14.300 USB).',
      net: { days: [0, 1, 2, 3, 4, 5, 6], utcTime: '16:00', alertEnabled: false, alertLeadMin: 10 },
    },
    {
      name: 'Hurricane Watch Net',
      rxMhz: 14.325,
      mode: 'USB',
      kind: 'hfnet',
      notes: 'Activated during tropical systems (also 7.268 LSB). Not a scheduled daily net.',
    },
    {
      name: 'SATERN Intl Net',
      rxMhz: 14.265,
      mode: 'USB',
      kind: 'hfnet',
      notes: 'Salvation Army emergency net — activated during disasters.',
    },
  ],
}

export const STARTER_PACKS: Pack[] = [CALLING, DIGITAL, POTA, NETS]

/** The fields a pack is the authority on, as a patch against a row it still owns.
 * Everything absent from the pack entry is patched to `undefined` so a correction
 * that REMOVES a note/tone clears it rather than leaving the stale value behind.
 * User-owned state is not in here and survives: id, groups, favorite, lastUsedUtc —
 * and, for a net, the operator's own reminder prefs (the pack owns WHEN the net
 * meets; the operator owns whether they're reminded and how early). */
function packContentPatch(pm: PackMemory, existing: Memory): Partial<Memory> {
  const patch: Partial<Memory> = {
    name: pm.name,
    kind: pm.kind,
    rxMhz: pm.rxMhz,
    mode: pm.mode,
    offsetDir: pm.offsetDir,
    offsetMhz: pm.offsetMhz,
    txMhz: pm.txMhz,
    toneMode: pm.toneMode,
    ctcssEncHz: pm.ctcssEncHz,
    ctcssDecHz: pm.ctcssDecHz,
    dtcsCode: pm.dtcsCode,
    dtcsRxCode: pm.dtcsRxCode,
    dtcsPol: pm.dtcsPol,
    notes: pm.notes,
    callsign: pm.callsign,
    grid: pm.grid,
    skip: pm.skip,
    net: pm.net
      ? {
          ...pm.net,
          alertEnabled: existing.net?.alertEnabled ?? pm.net.alertEnabled,
          alertLeadMin: existing.net?.alertLeadMin ?? pm.net.alertLeadMin,
        }
      : undefined,
  }
  return patch
}

/** Install (or re-install) a pack into the bank: its channels land (deduped on
 * freq+mode+tone) in a group named after the pack, tagged source 'curated'.
 *
 * Idempotent, and a re-install RECONCILES — a channel already present is refreshed
 * from the pack, so a corrected net time or note in a later Nexus release actually
 * reaches an operator who installed the pack earlier. Only rows the pack still owns
 * (`source: 'curated'`) are touched: editing a row in the Memories UI stamps it
 * `source: 'user'` and a pack never overwrites it again.
 *
 * KNOWN LIMIT: identity is freq+mode+tone, so a pack entry whose FREQUENCY changes
 * reads as a new channel — the corrected row is added and the stale one is left in
 * place for the operator to delete. Fixing that needs a stable per-entry id, which
 * no pack has yet needed (the volatile field is a net's time, which is content-only
 * and reconciles correctly). Revisit if a pack ever moves a frequency.
 *
 * Returns the counts added + updated (both 0 = genuinely already up to date). */
export function importPack(
  bank: MemoriesBank,
  pack: Pack,
): { bank: MemoriesBank; added: number; updated: number } {
  let b = bank
  let group = b.groups.find((g) => g.name === pack.name)
  if (!group) {
    b = addGroup(b, pack.name)
    group = b.groups.find((g) => g.name === pack.name)
  }
  const gid = group?.id
  let added = 0
  let updated = 0
  for (const pm of pack.memories) {
    const probe = coerceMemory({ ...pm, id: 'probe' })
    if (!probe) continue
    const key = memoryKey(probe)
    const existing = b.memories.find((m) => memoryKey(m) === key)
    if (existing) {
      // A channel two packs share (e.g. FT8 14.074 in both Digital and POTA) is not
      // re-added, but it MUST still join THIS pack's group — otherwise the second pack's
      // group would silently be missing the shared channels.
      if (gid && !existing.groups.includes(gid)) {
        b = updateMemory(b, existing.id, { groups: [...existing.groups, gid] })
      }
      if (existing.source !== 'curated') continue // the operator owns this row now
      // Count an update only when the row actually changed, so the toast can't claim
      // work it didn't do. Compare after the group join above, which is not an update.
      const before = JSON.stringify(b.memories.find((m) => m.id === existing.id))
      b = updateMemory(b, existing.id, packContentPatch(pm, existing))
      if (JSON.stringify(b.memories.find((m) => m.id === existing.id)) !== before) updated++
    } else {
      b = addMemory(b, { ...pm, groups: gid ? [gid] : [], source: 'curated' })
      added++
    }
  }
  return { bank: b, added, updated }
}
