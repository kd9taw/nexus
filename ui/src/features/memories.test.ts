import { describe, expect, it } from 'vitest'
import {
  addGroup,
  addMemory,
  addMemoryDeduped,
  CHIRP_HEADER,
  coerceBank,
  coerceMemory,
  deleteGroup,
  deleteMemory,
  derivedName,
  emptyBank,
  hotkeyRecallTarget,
  memoryKey,
  migrateV1Channel,
  moveMemory,
  parseChirpCsv,
  planRecall,
  saveFavoriteFromDial,
  setMemoryGroups,
  toChirpCsv,
  toggleFavorite,
  updateMemory,
  type Memory,
} from './memories'

const mem = (over: Partial<Memory> = {}): Memory => ({
  id: 'm-test-1',
  name: 'Test',
  kind: 'other',
  rxMhz: 14.074,
  mode: 'USB',
  groups: [],
  favorite: false,
  source: 'user',
  ...over,
})

describe('v1 → v2 migration', () => {
  it('maps a plain v1 channel and marks it favorite (old list was always visible)', () => {
    const m = migrateV1Channel({ id: 'ch-1', label: 'Net', freqMhz: 3.965, mode: 'LSB' })
    expect(m).toMatchObject({ id: 'ch-1', name: 'Net', rxMhz: 3.965, mode: 'LSB', favorite: true, kind: 'other' })
  })

  it('maps v1 repeater fields (shift/offsetHz/toneHz) onto the v2 model', () => {
    const m = migrateV1Channel({
      id: 'ch-2',
      label: 'W9ABC',
      freqMhz: 146.94,
      mode: 'FM',
      rptrShift: 'minus',
      offsetHz: 600_000,
      toneHz: 103.5,
    })
    expect(m).toMatchObject({
      kind: 'repeater',
      offsetDir: 'minus',
      offsetMhz: 0.6,
      toneMode: 'tone',
      ctcssEncHz: 103.5,
    })
  })

  it('classifies a shift-less FM v1 channel as simplex and drops junk', () => {
    expect(migrateV1Channel({ label: 'Calling', freqMhz: 146.52, mode: 'FM' })?.kind).toBe('simplex')
    expect(migrateV1Channel({ freqMhz: -1, mode: 'FM' })).toBeNull()
    expect(migrateV1Channel({ freqMhz: 146.52 })).toBeNull()
    expect(migrateV1Channel(null)).toBeNull()
  })
})

describe('coercion', () => {
  it('requires a positive freq and a mode, repairs the rest', () => {
    expect(coerceMemory({ rxMhz: 0, mode: 'FM' })).toBeNull()
    expect(coerceMemory({ rxMhz: 146.52, mode: '' })).toBeNull()
    const m = coerceMemory({ rxMhz: 146.52, mode: 'FM' })
    expect(m?.name).toBe(derivedName(146.52, 'FM'))
    expect(m?.kind).toBe('other')
    expect(m?.id).toBeTruthy()
  })

  it('keeps only a valid net schedule', () => {
    const good = coerceMemory({ rxMhz: 3.965, mode: 'LSB', net: { days: [0], utcTime: '18:30' } })
    expect(good?.net).toMatchObject({ days: [0], utcTime: '18:30', alertEnabled: false, alertLeadMin: 10 })
    const bad = coerceMemory({ rxMhz: 3.965, mode: 'LSB', net: { days: [0], utcTime: 'six pm' } })
    expect(bad?.net).toBeUndefined()
  })

  it('re-mints duplicate ids across the bank', () => {
    const bank = coerceBank({
      memories: [mem({ id: 'dup' }), mem({ id: 'dup', rxMhz: 7.2 })],
      groups: [],
    })
    expect(bank.memories).toHaveLength(2)
    expect(bank.memories[0].id).not.toBe(bank.memories[1].id)
  })
})

describe('operations', () => {
  it('add / update / delete / favorite / move round-trip', () => {
    let bank = emptyBank()
    bank = addMemory(bank, { rxMhz: 146.52, mode: 'FM', name: 'Calling' })
    bank = addMemory(bank, { rxMhz: 14.074, mode: 'FT8' })
    expect(bank.memories).toHaveLength(2)
    const id = bank.memories[0].id
    bank = updateMemory(bank, id, { name: 'NA Calling' })
    expect(bank.memories[0].name).toBe('NA Calling')
    // an invalid edit leaves the row unchanged rather than dropping it
    bank = updateMemory(bank, id, { rxMhz: NaN })
    expect(bank.memories[0].rxMhz).toBe(146.52)
    bank = toggleFavorite(bank, id)
    expect(bank.memories[0].favorite).toBe(true)
    bank = moveMemory(bank, id, 1)
    expect(bank.memories[1].id).toBe(id)
    bank = moveMemory(bank, id, 1) // at the end — no-op
    expect(bank.memories[1].id).toBe(id)
    bank = deleteMemory(bank, id)
    expect(bank.memories).toHaveLength(1)
  })

  it('dedupes on freq+mode+tone', () => {
    let bank = emptyBank()
    const r1 = addMemoryDeduped(bank, { rxMhz: 146.94, mode: 'FM', ctcssEncHz: 103.5 })
    bank = r1.bank
    const r2 = addMemoryDeduped(bank, { rxMhz: 146.94, mode: 'FM', ctcssEncHz: 103.5, name: 'Again' })
    expect(r1.added).toBe(true)
    expect(r2.added).toBe(false)
    expect(r2.bank.memories).toHaveLength(1)
    // a different tone IS a different channel
    const r3 = addMemoryDeduped(r2.bank, { rxMhz: 146.94, mode: 'FM', ctcssEncHz: 91.5 })
    expect(r3.added).toBe(true)
    expect(memoryKey(r3.bank.memories[0])).not.toBe(memoryKey(r3.bank.memories[1]))
  })

  it('groups: add/rename-safe/delete removes membership but keeps memories', () => {
    let bank = emptyBank()
    bank = addGroup(bank, '2m Repeaters')
    const gid = bank.groups[0].id
    bank = addMemory(bank, { rxMhz: 146.94, mode: 'FM' })
    bank = setMemoryGroups(bank, bank.memories[0].id, [gid, 'nonexistent'])
    expect(bank.memories[0].groups).toEqual([gid]) // unknown ids filtered
    bank = deleteGroup(bank, gid)
    expect(bank.groups).toHaveLength(0)
    expect(bank.memories).toHaveLength(1)
    expect(bank.memories[0].groups).toEqual([])
  })
})

describe('planRecall (auto-switch routing)', () => {
  it('routes CW → cw, digital → operate, voice → phone', () => {
    expect(planRecall(mem({ mode: 'CW' })).view).toBe('cw')
    expect(planRecall(mem({ mode: 'FT8' })).view).toBe('operate')
    expect(planRecall(mem({ mode: 'FT4' })).view).toBe('operate')
    expect(planRecall(mem({ mode: 'USB' })).view).toBe('phone')
    expect(planRecall(mem({ mode: 'FM' })).view).toBe('phone')
  })

  it('an FM repeater recall carries the full rig plumbing (shift + tone + offset)', () => {
    const plan = planRecall(
      mem({
        mode: 'FM',
        kind: 'repeater',
        offsetDir: 'minus',
        offsetMhz: 0.6,
        toneMode: 'tone',
        ctcssEncHz: 103.5,
      }),
    )
    expect(plan.settingsPatch).toEqual({
      phoneMode: 'fm',
      rptrShift: 'minus',
      rptrOffsetOverrideHz: 600_000,
      ctcssToneHz: 103.5,
    })
  })

  it('an odd split derives direction + offset from txMhz', () => {
    const plan = planRecall(
      mem({ mode: 'FM', kind: 'repeater', offsetDir: 'split', txMhz: 147.3, rxMhz: 146.94 }),
    )
    expect(plan.settingsPatch?.rptrShift).toBe('plus')
    expect(plan.settingsPatch?.rptrOffsetOverrideHz).toBe(360_000)
  })

  it('an SSB recall flips phone to ssb and clears no repeater fields', () => {
    const plan = planRecall(mem({ mode: 'LSB', rxMhz: 3.965 }))
    expect(plan.view).toBe('phone')
    expect(plan.settingsPatch).toEqual({ phoneMode: 'ssb' })
  })
})

describe('CHIRP CSV round-trip', () => {
  it('exports the canonical header and a correct repeater row', () => {
    const csv = toChirpCsv([
      mem({
        name: 'W9ABC',
        mode: 'FM',
        kind: 'repeater',
        rxMhz: 146.94,
        offsetDir: 'minus',
        offsetMhz: 0.6,
        toneMode: 'tone',
        ctcssEncHz: 103.5,
      }),
    ])
    const [header, row] = csv.trim().split('\r\n')
    expect(header).toBe(CHIRP_HEADER)
    const f = row.split(',')
    expect(f[0]).toBe('1') // Location is 1-based
    expect(f[1]).toBe('W9ABC')
    expect(f[2]).toBe('146.940000')
    expect(f[3]).toBe('-')
    expect(f[4]).toBe('0.600000')
    expect(f[5]).toBe('Tone')
    expect(f[6]).toBe('103.5')
    expect(f[10]).toBe('FM')
  })

  it('round-trips a repeater, an odd split, and a non-CHIRP mode (FT8 via comment tag)', () => {
    const original = [
      mem({
        name: 'W9ABC',
        mode: 'FM',
        kind: 'repeater',
        rxMhz: 146.94,
        offsetDir: 'minus',
        offsetMhz: 0.6,
        toneMode: 'tone',
        ctcssEncHz: 103.5,
      }),
      mem({ id: 'm2', name: 'Odd', mode: 'FM', kind: 'repeater', rxMhz: 53.03, offsetDir: 'split', txMhz: 52.03 }),
      mem({ id: 'm3', name: 'FT8 20m', mode: 'FT8', rxMhz: 14.074 }),
    ]
    const back = parseChirpCsv(toChirpCsv(original))
    expect(back).toHaveLength(3)
    expect(back[0]).toMatchObject({
      name: 'W9ABC',
      mode: 'FM',
      kind: 'repeater',
      rxMhz: 146.94,
      offsetDir: 'minus',
      offsetMhz: 0.6,
      toneMode: 'tone',
      ctcssEncHz: 103.5,
    })
    expect(back[1]).toMatchObject({ offsetDir: 'split', txMhz: 52.03 })
    expect(back[2]).toMatchObject({ mode: 'FT8', kind: 'digital', rxMhz: 14.074 })
  })

  it('escapes and re-parses names with commas/quotes', () => {
    const back = parseChirpCsv(toChirpCsv([mem({ name: 'Net, "the big one"', rxMhz: 7.2, mode: 'LSB' })]))
    expect(back[0].name).toBe('Net, "the big one"')
  })

  it('tolerates junk rows and a non-CHIRP file', () => {
    expect(parseChirpCsv('hello\nworld')).toEqual([])
    const csv = `${CHIRP_HEADER}\r\n1,Bad,not-a-freq,,0.000000,,88.5,88.5,023,NN,FM,5.00,,,,,,\r\n`
    expect(parseChirpCsv(csv)).toEqual([])
  })

  it('exports the band-standard offset for a plus/minus repeater with no explicit offset', () => {
    // 2m → 0.6, 70cm → 5 — CHIRP takes Offset literally, so a 0 would be unusable.
    const csv = toChirpCsv([
      mem({ rxMhz: 146.94, mode: 'FM', offsetDir: 'minus', offsetMhz: undefined }),
      mem({ rxMhz: 442.0, mode: 'FM', offsetDir: 'plus', offsetMhz: undefined }),
    ])
    const rows = csv.trim().split('\r\n').slice(1)
    expect(rows[0].split(',')[4]).toBe('0.600000')
    expect(rows[1].split(',')[4]).toBe('5.000000')
  })

  it('imports a TSQL access tone from cToneFreq, not rToneFreq (CHIRP quirk)', () => {
    // rToneFreq is the default 88.5; the REAL squelch tone (123.0) is in cToneFreq.
    const csv = `${CHIRP_HEADER}\r\n1,Rptr,146.940000,-,0.600000,TSQL,88.5,123.0,023,NN,FM,5.00,,,,,,\r\n`
    const [m] = parseChirpCsv(csv)
    expect(m.toneMode).toBe('tsql')
    expect(m.ctcssEncHz).toBe(123.0)
    expect(m.ctcssDecHz).toBe(123.0)
  })

  it('treats a bracketed comment as a mode ONLY when it is a known digital mode', () => {
    const digital = `${CHIRP_HEADER}\r\n1,FT8,14.074000,,0.000000,,88.5,88.5,023,NN,USB,5.00,,[FT8] common,,,,\r\n`
    expect(parseChirpCsv(digital)[0].mode).toBe('FT8')
    // A legit bracketed comment must NOT be swallowed into the mode.
    const ares = `${CHIRP_HEADER}\r\n1,Net,7.232000,,0.000000,,88.5,88.5,023,NN,USB,5.00,,[ARES] Sunday check-in,,,,\r\n`
    const m = parseChirpCsv(ares)[0]
    expect(m.mode).toBe('USB')
    expect(m.notes).toBe('[ARES] Sunday check-in')
  })

  it('round-trips a comment containing an embedded newline (quoted-field CSV)', () => {
    const original = [mem({ name: 'Multi', rxMhz: 7.2, mode: 'LSB', notes: 'line one\nline two' })]
    const back = parseChirpCsv(toChirpCsv(original))
    expect(back).toHaveLength(1)
    expect(back[0].notes).toBe('line one\nline two')
  })

  it('round-trips a DIGITAL mode with a MULTI-LINE note (tag regex spans the newline)', () => {
    const original = [mem({ mode: 'FT8', kind: 'digital', rxMhz: 14.074, notes: 'line one\nline two' })]
    const back = parseChirpCsv(toChirpCsv(original))
    expect(back).toHaveLength(1)
    expect(back[0].mode).toBe('FT8')
    expect(back[0].kind).toBe('digital')
    expect(back[0].notes).toBe('line one\nline two')
  })

  it('does NOT un-tag a bracketed comment naming a real CHIRP mode (RTTY) on a USB row', () => {
    // We never export RTTY as USB+[RTTY] (RTTY is its own CHIRP mode), so a USB memory whose
    // comment merely mentions [RTTY] must stay USB, not flip to RTTY.
    const csv = `${CHIRP_HEADER}\r\n1,Net,14.100000,,0.000000,,88.5,88.5,023,NN,USB,5.00,,[RTTY] also here,,,,\r\n`
    const m = parseChirpCsv(csv)[0]
    expect(m.mode).toBe('USB')
    expect(m.notes).toBe('[RTTY] also here')
  })
})

describe('saveFavoriteFromDial (cockpit ＋)', () => {
  it('adds a new favorite', () => {
    const { bank, result } = saveFavoriteFromDial(emptyBank(), { rxMhz: 146.52, mode: 'FM' })
    expect(result).toBe('added')
    expect(bank.memories).toHaveLength(1)
    expect(bank.memories[0].favorite).toBe(true)
  })

  it('stars an equivalent non-favorite instead of a silent no-op', () => {
    const base = addMemory(emptyBank(), { rxMhz: 146.52, mode: 'FM', favorite: false })
    const { bank, result } = saveFavoriteFromDial(base, { rxMhz: 146.52, mode: 'FM' })
    expect(result).toBe('starred')
    expect(bank.memories).toHaveLength(1)
    expect(bank.memories[0].favorite).toBe(true)
  })

  it('is a no-op when an equivalent favorite already exists', () => {
    const base = addMemory(emptyBank(), { rxMhz: 146.52, mode: 'FM', favorite: true })
    const { bank, result } = saveFavoriteFromDial(base, { rxMhz: 146.52, mode: 'FM' })
    expect(result).toBe('exists')
    expect(bank.memories).toHaveLength(1)
  })
})

describe('hotkeyRecallTarget — Ctrl+1..9 quick recall', () => {
  // A bank whose favorites (in bank order) are fav-A, fav-B; a non-favorite sits between
  // them to prove indexing counts favorites only, in bank order.
  const bank = {
    version: 2 as const,
    groups: [],
    memories: [
      mem({ id: 'a', name: 'Fav A', favorite: true }),
      mem({ id: 'x', name: 'Not fav', favorite: false }),
      mem({ id: 'b', name: 'Fav B', favorite: true }),
    ],
  }
  const chord = (over: Partial<{ ctrlKey: boolean; altKey: boolean; metaKey: boolean; shiftKey: boolean; code: string }> = {}) => ({
    ctrlKey: true,
    altKey: false,
    metaKey: false,
    shiftKey: false,
    code: 'Digit1',
    ...over,
  })

  it('maps Ctrl+1 / Ctrl+2 to the 1st / 2nd favorite in bank order', () => {
    expect(hotkeyRecallTarget(chord({ code: 'Digit1' }), bank)?.id).toBe('a')
    expect(hotkeyRecallTarget(chord({ code: 'Digit2' }), bank)?.id).toBe('b')
  })

  it('returns null past the last favorite', () => {
    expect(hotkeyRecallTarget(chord({ code: 'Digit3' }), bank)).toBeNull()
  })

  it('requires Ctrl ALONE — Alt/Meta/Shift + digit are other rigs’ shortcuts', () => {
    expect(hotkeyRecallTarget(chord({ ctrlKey: false }), bank)).toBeNull()
    expect(hotkeyRecallTarget(chord({ altKey: true }), bank)).toBeNull() // Alt+1 = FT8 Tx
    expect(hotkeyRecallTarget(chord({ metaKey: true }), bank)).toBeNull()
    expect(hotkeyRecallTarget(chord({ shiftKey: true }), bank)).toBeNull()
  })

  it('ignores non-digit and numpad codes', () => {
    expect(hotkeyRecallTarget(chord({ code: 'KeyA' }), bank)).toBeNull()
    expect(hotkeyRecallTarget(chord({ code: 'Digit0' }), bank)).toBeNull()
    expect(hotkeyRecallTarget(chord({ code: 'Numpad1' }), bank)).toBeNull()
  })

  it('returns null when there are no favorites', () => {
    expect(hotkeyRecallTarget(chord({ code: 'Digit1' }), emptyBank())).toBeNull()
  })
})
