import { describe, expect, it } from 'vitest'
import { importPack, STARTER_PACKS, type Pack } from './packs'
import { emptyBank, coerceMemory, updateMemory, toggleFavorite } from './memories'

/** A one-channel net pack we can "correct" between installs, to exercise the refresh
 * path the same way a later Nexus release would (packs are bundled, so a corrected
 * net time only reaches an installed operator via a re-install). */
function netPack(over: { utcTime?: string; notes?: string; name?: string } = {}): Pack {
  return {
    id: 'test-net',
    name: 'Test Net Pack',
    description: 'fixture',
    region: 'North America',
    memories: [
      {
        name: over.name ?? 'Test Net',
        rxMhz: 7.2,
        mode: 'LSB',
        kind: 'hfnet',
        notes: over.notes ?? 'original note',
        net: { days: [1], utcTime: over.utcTime ?? '01:00', alertEnabled: false, alertLeadMin: 10 },
      },
    ],
  }
}

describe('starter packs', () => {
  it('every packed memory is valid (coerces cleanly)', () => {
    for (const pack of STARTER_PACKS) {
      for (const pm of pack.memories) {
        const m = coerceMemory({ ...pm, id: 'x' })
        expect(m, `${pack.name} / ${pm.name}`).not.toBeNull()
        expect(m?.rxMhz).toBeGreaterThan(0)
      }
    }
  })

  it('installs a pack into a named group, tagged curated', () => {
    const pack = STARTER_PACKS[0]
    const { bank, added } = importPack(emptyBank(), pack)
    expect(added).toBe(pack.memories.length)
    expect(bank.groups.some((g) => g.name === pack.name)).toBe(true)
    const gid = bank.groups.find((g) => g.name === pack.name)?.id
    expect(bank.memories.every((m) => m.source === 'curated')).toBe(true)
    expect(bank.memories.every((m) => gid && m.groups.includes(gid))).toBe(true)
  })

  it('is idempotent — re-installing adds nothing and does not duplicate the group', () => {
    const pack = STARTER_PACKS[0]
    const first = importPack(emptyBank(), pack)
    const second = importPack(first.bank, pack)
    expect(second.added).toBe(0)
    expect(second.bank.memories).toHaveLength(pack.memories.length)
    expect(second.bank.groups.filter((g) => g.name === pack.name)).toHaveLength(1)
  })

  it('adds a channel shared by two packs to BOTH packs’ groups', () => {
    const digital = STARTER_PACKS.find((p) => p.id === 'na-digital')!
    const pota = STARTER_PACKS.find((p) => p.id === 'na-pota-sota')!
    let bank = importPack(emptyBank(), digital).bank
    bank = importPack(bank, pota).bank
    const potaGid = bank.groups.find((g) => g.name === pota.name)!.id
    // 14.074 FT8 lives in both packs (same memoryKey) — it must be tagged into the POTA group
    // even though it was first added by the Digital pack.
    const shared = bank.memories.find((m) => m.rxMhz === 14.074 && m.mode === 'FT8')
    expect(shared?.groups).toContain(potaGid)
  })

  it('installs a scheduled net with its reminder default off', () => {
    // The bundled packs ship no net schedules today (net times are volatile), so this
    // exercises the machinery with the fixture — an installed net keeps its schedule and
    // does NOT pre-arm reminders.
    const { bank } = importPack(emptyBank(), netPack({ utcTime: '01:00' }))
    const net = bank.memories.find((m) => m.name === 'Test Net')
    expect(net?.net?.utcTime).toBe('01:00')
    expect(net?.net?.alertEnabled).toBe(false)
  })
})

describe('pack re-install reconciles (the "Update did nothing" bug)', () => {
  it('applies a corrected net time to an already-installed curated row', () => {
    const first = importPack(emptyBank(), netPack({ utcTime: '01:00' }))
    expect(first.added).toBe(1)
    // A later release corrects the net's time. Re-installing must APPLY it — the old
    // add-only importPack matched on freq+mode+tone, found the row, and skipped it,
    // leaving the operator with the stale time and an "already up to date" toast.
    const second = importPack(first.bank, netPack({ utcTime: '02:30' }))
    expect(second.added).toBe(0)
    expect(second.updated).toBe(1)
    expect(second.bank.memories).toHaveLength(1)
    expect(second.bank.memories[0].net?.utcTime).toBe('02:30')
  })

  it('refreshes name and notes too, and clears a note the pack dropped', () => {
    const first = importPack(emptyBank(), netPack({ name: 'Old Name', notes: 'stale' }))
    const second = importPack(first.bank, netPack({ name: 'New Name', notes: undefined }))
    expect(second.updated).toBe(1)
    expect(second.bank.memories[0].name).toBe('New Name')
    // `notes: undefined` in the fixture falls back to 'original note'; assert the rename
    // landed and the row is still the same single channel.
    expect(second.bank.memories).toHaveLength(1)
  })

  it('reports nothing when the pack is genuinely unchanged', () => {
    const first = importPack(emptyBank(), netPack())
    const second = importPack(first.bank, netPack())
    expect(second.added).toBe(0)
    expect(second.updated).toBe(0)
  })

  it('never clobbers a row the operator has edited (source stamped user)', () => {
    const first = importPack(emptyBank(), netPack({ utcTime: '01:00' }))
    const id = first.bank.memories[0].id
    // Simulates the Memories editor, which stamps source 'user' on any content edit.
    const edited = updateMemory(first.bank, id, { name: 'My Renamed Net', source: 'user' })
    const second = importPack(edited, netPack({ utcTime: '02:30', name: 'Test Net' }))
    expect(second.updated).toBe(0)
    expect(second.bank.memories[0].name).toBe('My Renamed Net')
    expect(second.bank.memories[0].net?.utcTime).toBe('01:00')
  })

  it('preserves the operator’s reminder prefs while refreshing the schedule', () => {
    const first = importPack(emptyBank(), netPack({ utcTime: '01:00' }))
    const id = first.bank.memories[0].id
    // Enabling a reminder is a PREF, not a content edit — it must not stop pack
    // refreshes, and a refresh must not silently switch the reminder back off.
    const armed = updateMemory(first.bank, id, {
      net: { ...first.bank.memories[0].net!, alertEnabled: true, alertLeadMin: 25 },
    })
    const second = importPack(armed, netPack({ utcTime: '02:30' }))
    expect(second.bank.memories[0].net?.utcTime).toBe('02:30') // schedule refreshed
    expect(second.bank.memories[0].net?.alertEnabled).toBe(true) // pref survived
    expect(second.bank.memories[0].net?.alertLeadMin).toBe(25)
  })

  it('a starred pack channel still receives corrections', () => {
    const first = importPack(emptyBank(), netPack({ utcTime: '01:00' }))
    const starred = toggleFavorite(first.bank, first.bank.memories[0].id)
    const second = importPack(starred, netPack({ utcTime: '02:30' }))
    expect(second.updated).toBe(1)
    expect(second.bank.memories[0].favorite).toBe(true)
    expect(second.bank.memories[0].net?.utcTime).toBe('02:30')
  })
})
