// @vitest-environment jsdom
//
// Storage scoping, exercised against a real Storage: does a second window actually stay
// out of the first's saved layout, and does station-level state actually still reach
// every window? The classification itself (which key is which) and the call-site scan
// live in src/storage-scope.test.ts, which needs no DOM.
import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import { act, renderHook } from '@testing-library/react'
import { scopedKey, surfaceGet, surfaceId, surfaceSet, windowInstance } from './windowScope'
import { readSeen, writeSeen } from '../seenSet'
import { usePaneWidths } from '../usePaneWidths'
import { useScale } from '../useScale'
import { loadWatchlist, newWatchFilter, saveWatchlist } from '../watchlist'

/** Run `fn` as if this webview were the given surface. */
function asSurface<T>(instance: string, fn: () => T): T {
  window.history.replaceState({}, '', instance === 'main' ? '/' : `/?panel=needed&instance=${instance}`)
  try {
    return fn()
  } finally {
    window.history.replaceState({}, '', '/')
  }
}

beforeEach(() => {
  localStorage.clear()
  sessionStorage.clear()
})
afterEach(() => {
  window.history.replaceState({}, '', '/')
})

describe('the surface this window is', () => {
  it('reads main / w<n> / r<id> off the URL', () => {
    expect(windowInstance()).toBe('main')
    expect(asSurface('w2', windowInstance)).toBe('w2')
    expect(asSurface('r7', windowInstance)).toBe('r7')
    // A malformed token is REJECTED, never coerced — `r02` must not become a second
    // surface for radio 2 (chains.rs makes the same promise for window labels).
    window.history.replaceState({}, '', '/?panel=waterfall&instance=r02')
    expect(windowInstance()).toBe('main')
  })

  /**
   * TWO POP-OUTS MUST NOT COLLIDE WITH EACH OTHER, not just with the main window.
   *
   * This is the bug this file was reviewed into existence for: keying on the instance
   * ALONE gave every pop-out one token, because `open_panel_window` sends no `&instance=`
   * for a `main` surface and every pop-out openable today is `main`. So a torn-off
   * waterfall and a torn-off Operate (which embeds a waterfall) shared one zoom key, and
   * the two band maps shared one spot-legend key. Relocating the collision is not fixing
   * it — the surface is (view, instance), and the VIEW is what discriminates today.
   */
  it('gives two different pop-outs two different surfaces', () => {
    window.history.replaceState({}, '', '/?panel=waterfall')
    expect(surfaceId()).toBe('waterfall')
    window.history.replaceState({}, '', '/?panel=operate')
    expect(surfaceId()).toBe('operate')
    window.history.replaceState({}, '', '/')
    expect(surfaceId()).toBe('main')

    // …and the keys they produce are genuinely distinct, which is the property that matters.
    const keyOf = (search: string) => {
      window.history.replaceState({}, '', search)
      return scopedKey('nexus.waterfall.zoom', 'surface')
    }
    const wf = keyOf('/?panel=waterfall')
    const op = keyOf('/?panel=operate')
    const bare = keyOf('/')
    expect(new Set([wf, op, bare]).size).toBe(3)
    expect(bare).toBe('nexus.waterfall.zoom') // main stays BARE — zero migration
  })

  it('still separates instances of the SAME view, for when w<n> is un-gated', () => {
    window.history.replaceState({}, '', '/?panel=needed')
    const a = scopedKey('neededFilters', 'surface')
    window.history.replaceState({}, '', '/?panel=needed&instance=w2')
    const b = scopedKey('neededFilters', 'surface')
    expect(a).not.toBe(b)
  })
})

describe('isolation: a second window cannot overwrite the first', () => {
  it('writes a per-surface key to its own slot and leaves the main one alone', () => {
    surfaceSet('neededFilters', 'MAIN')
    asSurface('w2', () => surfaceSet('neededFilters', 'SECOND'))
    expect(localStorage.getItem('neededFilters')).toBe('MAIN')
    // The suffix carries the whole SURFACE (view + instance), not the instance alone —
    // `asSurface` opens the `needed` view, so `w2` of it is `needed.w2`. Pinned as a
    // literal because the exact string IS the contract with what is already on disk.
    expect(localStorage.getItem('neededFilters.needed.w2')).toBe('SECOND')
    expect(Object.keys(localStorage)).not.toContain('neededFilters.w2')
    expect(surfaceGet('neededFilters')).toBe('MAIN')
    expect(asSurface('w2', () => surfaceGet('neededFilters'))).toBe('SECOND')
  })

  it('keeps two extra surfaces apart from each other, not just from main', () => {
    asSurface('w2', () => surfaceSet('nexus.split.operate.waterfall', '40'))
    asSurface('w3', () => surfaceSet('nexus.split.operate.waterfall', '15'))
    expect(asSurface('w2', () => surfaceGet('nexus.split.operate.waterfall'))).toBe('40')
    expect(asSurface('w3', () => surfaceGet('nexus.split.operate.waterfall'))).toBe('15')
  })

  it('inherits the main window once, then diverges for good', () => {
    // Why inherit at all: nexus.connect.projection exists PRECISELY so a torn-off map
    // opens on the globe you were already using. A cold per-surface read would reset it
    // to the intent preset (pota's is the flat world map) — the exact bug that key was
    // added to fix. Inheriting is free in the other direction: the first write in the
    // pop-out makes it that window's own.
    surfaceSet('nexus.connect.projection', 'globe')
    expect(asSurface('w2', () => surfaceGet('nexus.connect.projection'))).toBe('globe')
    asSurface('w2', () => surfaceSet('nexus.connect.projection', 'world'))
    surfaceSet('nexus.connect.projection', 'aeqd')
    expect(asSurface('w2', () => surfaceGet('nexus.connect.projection'))).toBe('world')
    expect(surfaceGet('nexus.connect.projection')).toBe('aeqd')
  })

  it('never invents a suffixed key for the main surface', () => {
    // The zero-migration guarantee at the storage layer, not just the key layer: an
    // upgraded operator's disk must gain no `.main` twin of anything.
    surfaceSet('nexus.view', 'logbook')
    surfaceSet('tempo-right-rail-w', '340')
    expect(Object.keys(localStorage).sort()).toEqual(['nexus.view', 'tempo-right-rail-w'])
  })

  it('survives storage being blocked, on both the read and the write', () => {
    const real = Object.getOwnPropertyDescriptor(window, 'localStorage')!
    Object.defineProperty(window, 'localStorage', {
      configurable: true,
      get() {
        throw new Error('blocked (read-only profile)')
      },
    })
    try {
      expect(() => surfaceSet('nexus.view', 'operate')).not.toThrow()
      expect(surfaceGet('nexus.view')).toBeNull()
    } finally {
      Object.defineProperty(window, 'localStorage', real)
    }
  })
})

describe('an upgraded operator: a pop-out cannot rewrite what is already on disk', () => {
  it('diffs the whole key set around a second surface driving the real hooks', () => {
    // The end-to-end form of the guarantee, and the one the key-string tests cannot give:
    // seed the store the way an upgrading operator's disk looks (bare keys, written by a
    // build that had never heard of instances), then run the ACTUAL production hooks on a
    // `w1` surface and diff. Driving the hooks matters — a scoping mistake shows up as a
    // rewritten value here, not as a theory about which call site was migrated.
    localStorage.setItem('tempo-right-rail-w', '420')
    localStorage.setItem('tempo-left-rail-w', '300')
    localStorage.setItem('nexus-ui-scale-mode', '125')
    const before = { ...localStorage }

    window.history.replaceState({}, '', '/?panel=needed') // a torn-off window = surface `needed`
    const rails = renderHook(() => usePaneWidths())
    act(() => rails.result.current.commitRight(280))
    const scale = renderHook(() => useScale())
    act(() => scale.result.current.setMode(75))

    // Main's values: untouched, byte for byte.
    expect(localStorage.getItem('tempo-right-rail-w')).toBe('420')
    expect(localStorage.getItem('tempo-left-rail-w')).toBe('300')
    expect(localStorage.getItem('nexus-ui-scale-mode')).toBe('125')
    expect(Object.fromEntries(Object.keys(before).map((k) => [k, localStorage.getItem(k)]))).toEqual(
      before,
    )
    // The pop-out's own choices landed beside them, never on top of them.
    expect(localStorage.getItem('tempo-right-rail-w.needed')).toBe('280')
    expect(localStorage.getItem('nexus-ui-scale-mode.needed')).toBe('75')
    // …and it invented nothing else: every new key is a twin suffixed with THIS surface.
    // Suffixing with the surface rather than a bare instance is what keeps a second pop-out
    // (say the torn-off waterfall) from landing on this one's keys as well as on main's.
    const added = Object.keys(localStorage).filter((k) => !(k in before))
    expect(added.every((k) => k.endsWith('.needed'))).toBe(true)
  })
})

describe('shared state stays shared', () => {
  it('a station-level list written in a pop-out is visible from the main window', () => {
    asSurface('w2', () => saveWatchlist([newWatchFilter('call', 'T33T')]))
    expect(localStorage.getItem('nexus.watchlist')).not.toBeNull()
    expect(loadWatchlist().map((f) => f.value)).toEqual(['T33T'])
  })

  it('a global key resolves identically from every surface', () => {
    for (const inst of ['main', 'w2', 'w3', 'r5']) {
      expect(scopedKey('nexus.dxped.alarms.fired', 'global', inst)).toBe('nexus.dxped.alarms.fired')
    }
  })

  it('a dedupe set cannot re-fire once per open window', () => {
    // The failure this prevents: three windows open, a DXpedition window opens, and the
    // operator gets the same "work it now" alert three times because each window thinks
    // it fired first. The seen set must key identically from every surface.
    asSurface('w2', () => writeSeen('nexus-journey-seen', new Set(['first-dx'])))
    expect(localStorage.getItem('nexus-journey-seen')).toBe('["first-dx"]')
    expect([...(readSeen('nexus-journey-seen') ?? [])]).toEqual(['first-dx'])
    expect([...(asSurface('w3', () => readSeen('nexus-journey-seen')) ?? [])]).toEqual(['first-dx'])
  })

  it('sessionStorage is already per-webview, so the localStorage half must carry dedupe', () => {
    // seenSet writes BOTH stores and reads their union. A newly opened window starts with
    // an empty session half, so if the localStorage half were ever scoped the dedupe would
    // degrade silently — "works on the machine you tested it on, re-toasts on his".
    writeSeen('tempo-achievements-seen', new Set(['dxcc-100']))
    sessionStorage.clear() // = a freshly opened second webview
    expect([...(readSeen('tempo-achievements-seen') ?? [])]).toEqual(['dxcc-100'])
  })
})
