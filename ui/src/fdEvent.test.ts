import { describe, it, expect } from 'vitest'
import {
  arrlFdSaturdayUnix,
  wfdSaturdayUnix,
  fdNextEvent,
  fdCountdownLabel,
  fdHeaderSubtitle,
} from './fdEvent'

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function unixToUtcDate(unix: number) {
  return new Date(unix * 1000)
}

function utcYMD(unix: number): string {
  const d = unixToUtcDate(unix)
  const p = (n: number) => String(n).padStart(2, '0')
  return `${d.getUTCFullYear()}-${p(d.getUTCMonth() + 1)}-${p(d.getUTCDate())}`
}

function utcHour(unix: number): number {
  return new Date(unix * 1000).getUTCHours()
}

function utcDow(unix: number): number {
  return new Date(unix * 1000).getUTCDay() // 6 = Saturday
}

// ---------------------------------------------------------------------------
// ARRL Field Day: 4th Saturday of June, 1800 UTC
// ---------------------------------------------------------------------------

describe('arrlFdSaturdayUnix', () => {
  it('2024 — 4th Saturday of June is June 22', () => {
    // June 2024: 1st=Sat, 8th=Sat, 15th=Sat, 22nd=Sat ✓
    const unix = arrlFdSaturdayUnix(2024)
    expect(utcYMD(unix)).toBe('2024-06-22')
    expect(utcHour(unix)).toBe(18)
    expect(utcDow(unix)).toBe(6) // Saturday
  })

  it('2025 — 4th Saturday of June is June 28', () => {
    // June 2025: 7th=Sat, 14th=Sat, 21st=Sat, 28th=Sat ✓
    const unix = arrlFdSaturdayUnix(2025)
    expect(utcYMD(unix)).toBe('2025-06-28')
    expect(utcHour(unix)).toBe(18)
    expect(utcDow(unix)).toBe(6)
  })

  it('2026 — 4th Saturday of June is June 27', () => {
    // June 2026: 6th=Sat, 13th=Sat, 20th=Sat, 27th=Sat ✓
    const unix = arrlFdSaturdayUnix(2026)
    expect(utcYMD(unix)).toBe('2026-06-27')
    expect(utcHour(unix)).toBe(18)
    expect(utcDow(unix)).toBe(6)
  })

  it('2027 — 4th Saturday of June is June 26', () => {
    // June 2027: 5th=Sat, 12th=Sat, 19th=Sat, 26th=Sat ✓
    const unix = arrlFdSaturdayUnix(2027)
    expect(utcYMD(unix)).toBe('2027-06-26')
    expect(utcHour(unix)).toBe(18)
    expect(utcDow(unix)).toBe(6)
  })

  it('never returns July (4th Saturday must be in June)', () => {
    // Test several years to be sure we never spill into July
    for (let y = 2020; y <= 2035; y++) {
      const d = unixToUtcDate(arrlFdSaturdayUnix(y))
      expect(d.getUTCMonth()).toBe(5) // June = 5 (0-indexed)
    }
  })
})

// ---------------------------------------------------------------------------
// Winter Field Day: last Saturday of January, 1600 UTC
// ---------------------------------------------------------------------------

describe('wfdSaturdayUnix', () => {
  it('2024 — last Saturday of January is January 27', () => {
    // Jan 2024: 6=Sat,13=Sat,20=Sat,27=Sat → last=27 ✓
    const unix = wfdSaturdayUnix(2024)
    expect(utcYMD(unix)).toBe('2024-01-27')
    expect(utcHour(unix)).toBe(16)
    expect(utcDow(unix)).toBe(6)
  })

  it('2025 — last Saturday of January is January 25', () => {
    // Jan 2025: 4=Sat,11=Sat,18=Sat,25=Sat → last=25 ✓
    const unix = wfdSaturdayUnix(2025)
    expect(utcYMD(unix)).toBe('2025-01-25')
    expect(utcHour(unix)).toBe(16)
    expect(utcDow(unix)).toBe(6)
  })

  it('2026 — last FULL weekend: Jan 24 (Sat 31 spills its Sunday into Feb)', () => {
    // Jan 2026 Saturdays: 3,10,17,24,31. Sat 31's Sunday is Feb 1 → NOT a full
    // January weekend; WFDA actually ran 2026 on Jan 24–25. The bare
    // last-Saturday rule was a week late.
    const unix = wfdSaturdayUnix(2026)
    expect(utcYMD(unix)).toBe('2026-01-24')
    expect(utcHour(unix)).toBe(16)
    expect(utcDow(unix)).toBe(6)
  })

  it('2027 — Sat 30 + Sun 31 both in January = full weekend', () => {
    expect(utcYMD(wfdSaturdayUnix(2027))).toBe('2027-01-30')
  })

  it('full-weekend rule: the Sunday is ALWAYS in January too', () => {
    for (let y = 2020; y <= 2035; y++) {
      const sunday = unixToUtcDate((wfdSaturdayUnix(y) + 86_400))
      expect(sunday.getUTCMonth()).toBe(0)
    }
  })

  it('never returns February', () => {
    for (let y = 2020; y <= 2035; y++) {
      const d = unixToUtcDate(wfdSaturdayUnix(y))
      expect(d.getUTCMonth()).toBe(0) // January = 0
    }
  })
})

// ---------------------------------------------------------------------------
// fdNextEvent — year boundary behavior
// ---------------------------------------------------------------------------

describe('fdNextEvent', () => {
  it('returns this-year ARRL FD when the event has not yet started', () => {
    // 2026-06-01 UTC — before the 4th Saturday (Jun 27)
    const now = new Date(Date.UTC(2026, 5, 1, 0, 0, 0))
    const ev = fdNextEvent(now, 'arrlfd')
    expect(ev.year).toBe(2026)
    expect(ev.label).toBe('ARRL Field Day')
    expect(utcYMD(ev.startUnix)).toBe('2026-06-27')
  })

  it('returns this-year ARRL FD while the event is active', () => {
    // 2026-06-27 21:00 UTC — inside the 24-hour window
    const now = new Date(Date.UTC(2026, 5, 27, 21, 0, 0))
    const ev = fdNextEvent(now, 'arrlfd')
    expect(ev.year).toBe(2026)
    expect(utcYMD(ev.startUnix)).toBe('2026-06-27')
  })

  it('returns NEXT year ARRL FD after the event ends', () => {
    // 2026-06-29 00:00 UTC — event ended (startUnix + 24h = Jun 28 18:00 UTC)
    const now = new Date(Date.UTC(2026, 5, 29, 0, 0, 0))
    const ev = fdNextEvent(now, 'arrlfd')
    expect(ev.year).toBe(2027)
  })

  it('WFD: returns this-year when before January event', () => {
    const now = new Date(Date.UTC(2026, 0, 15, 0, 0, 0)) // Jan 15 — before Jan 24
    const ev = fdNextEvent(now, 'wfd')
    expect(ev.year).toBe(2026)
    expect(utcYMD(ev.startUnix)).toBe('2026-01-24')
  })

  it('WFD: returns next year when past January event', () => {
    // Feb 1 — past Jan 31 end (Jan 31 16:00 + 24h = Feb 1 16:00)
    const now = new Date(Date.UTC(2026, 1, 2, 0, 0, 0))
    const ev = fdNextEvent(now, 'wfd')
    expect(ev.year).toBe(2027)
    expect(ev.label).toBe('Winter Field Day')
  })

  it('event endUnix is exactly 24 hours after startUnix', () => {
    const now = new Date(Date.UTC(2025, 0, 1))
    const arrl = fdNextEvent(now, 'arrlfd')
    expect(arrl.endUnix - arrl.startUnix).toBe(24 * 3600)

    const wfd = fdNextEvent(now, 'wfd')
    expect(wfd.endUnix - wfd.startUnix).toBe(24 * 3600)
  })
})

// ---------------------------------------------------------------------------
// fdCountdownLabel
// ---------------------------------------------------------------------------

describe('fdCountdownLabel', () => {
  it('returns null when event is active', () => {
    const now = new Date(Date.UTC(2026, 5, 27, 20, 0, 0)) // inside 2026 FD
    const ev = fdNextEvent(now, 'arrlfd')
    expect(fdCountdownLabel(now, ev)).toBeNull()
  })

  it('returns "starts in N days" when more than 1 day away', () => {
    const now = new Date(Date.UTC(2026, 5, 1, 0, 0, 0)) // Jun 1
    const ev = fdNextEvent(now, 'arrlfd')
    const label = fdCountdownLabel(now, ev)
    expect(label).toMatch(/^starts in \d+ days$/)
    // Jun 1 → Jun 27 = 26 days
    expect(label).toBe('starts in 26 days')
  })

  it('returns "starts tomorrow" when 1 day away', () => {
    // 2026-06-26 17:00 UTC — 25h before Jun 27 18:00 = days=1 → "starts tomorrow"
    const now = new Date(Date.UTC(2026, 5, 26, 17, 0, 0))
    const ev = fdNextEvent(now, 'arrlfd')
    const label = fdCountdownLabel(now, ev)
    expect(label).toBe('starts tomorrow')
  })
})

// ---------------------------------------------------------------------------
// fdHeaderSubtitle
// ---------------------------------------------------------------------------

describe('fdHeaderSubtitle', () => {
  it('formats ARRL FD subtitle with date range and countdown', () => {
    const now = new Date(Date.UTC(2026, 5, 1, 0, 0, 0))
    const ev = fdNextEvent(now, 'arrlfd')
    const sub = fdHeaderSubtitle(now, ev)
    // Jun 27–28 (start Jun 27, end Jun 28 after 24h)
    expect(sub).toContain('ARRL Field Day')
    expect(sub).toContain('Jun')
    expect(sub).toContain('starts in')
  })

  it('formats WFD subtitle when active', () => {
    const now = new Date(Date.UTC(2026, 0, 24, 18, 0, 0)) // inside WFD (Jan 24–25)
    const ev = fdNextEvent(now, 'wfd')
    const sub = fdHeaderSubtitle(now, ev)
    expect(sub).toContain('Winter Field Day')
    expect(sub).toContain('active')
  })

  it('a full WFD weekend never spans into February', () => {
    // The corrected rule (BOTH days in January) means the subtitle range is
    // always single-month for WFD — the old last-Saturday rule produced a
    // bogus "Jan 31 – Feb 1" event a week after the real one.
    const now = new Date(Date.UTC(2026, 0, 1, 0, 0, 0))
    const ev = fdNextEvent(now, 'wfd')
    const sub = fdHeaderSubtitle(now, ev)
    expect(sub).toContain('Jan')
    expect(sub).not.toContain('Feb')
  })
})
