import { describe, it, expect } from 'vitest'
import {
  contestBucket,
  upcomingContests,
  formatRange,
  contestsLine,
  type ContestEvent,
} from './ContestCalendarPane'

// A fixed reference clock so the bucketing thresholds are deterministic.
const NOW = 1_783_382_400 // 2026-07-07 00:00:00 Z
const HOUR = 3600
const DAY = 86_400

const ev = (name: string, startUnix: number, endUnix: number, url?: string): ContestEvent => ({
  name,
  startUnix,
  endUnix,
  url,
})

describe('contestBucket', () => {
  it('flags a contest running across now', () => {
    expect(contestBucket(ev('Live', NOW - HOUR, NOW + HOUR), NOW)).toBe('now')
  })
  it('buckets by lead time: soon (<24h), week (<7d), later (>7d)', () => {
    expect(contestBucket(ev('Soon', NOW + 6 * HOUR, NOW + 8 * HOUR), NOW)).toBe('soon')
    expect(contestBucket(ev('Week', NOW + 3 * DAY, NOW + 3 * DAY + HOUR), NOW)).toBe('week')
    expect(contestBucket(ev('Later', NOW + 10 * DAY, NOW + 10 * DAY + HOUR), NOW)).toBe('later')
  })
  it('the 24h boundary is inclusive of soon', () => {
    expect(contestBucket(ev('Edge', NOW + 24 * HOUR, NOW + 25 * HOUR), NOW)).toBe('soon')
    expect(contestBucket(ev('Past', NOW + 24 * HOUR + 1, NOW + 25 * HOUR), NOW)).toBe('week')
  })
})

describe('upcomingContests', () => {
  it('drops already-ended contests and sorts soonest-first', () => {
    const list = [
      ev('Future', NOW + 3 * DAY, NOW + 3 * DAY + HOUR),
      ev('Ended', NOW - 2 * DAY, NOW - DAY), // endUnix < now → filtered out
      ev('NowRunning', NOW - HOUR, NOW + HOUR),
    ]
    const up = upcomingContests(list, NOW)
    expect(up.map((e) => e.name)).toEqual(['NowRunning', 'Future'])
  })
})

describe('formatRange (UTC)', () => {
  it('collapses the end date for a same-day window', () => {
    // 2026-07-07 00:00Z → 02:00Z
    expect(formatRange(ev('x', 1_783_382_400, 1_783_389_600))).toBe('Jul 7 0000Z → 0200Z')
  })
  it('shows both dates for a multi-day window', () => {
    // 2026-07-11 12:00Z → 2026-07-12 12:00Z
    expect(formatRange(ev('x', 1_783_771_200, 1_783_857_600))).toBe('Jul 11 1200Z → Jul 12 1200Z')
  })
})

describe('contestsLine', () => {
  it('is an honest loading hint when data is null', () => {
    expect(contestsLine(null, NOW)).toBe('Contest schedule loads once online.')
  })
  it('leads with a running contest', () => {
    const line = contestsLine([ev('IARU HF', NOW - HOUR, NOW + 6 * HOUR)], NOW)
    expect(line).toBe('On now: IARU HF (until 0600Z).')
  })
  it('otherwise names the next contest and its lead time', () => {
    const line = contestsLine([ev('Sprint', NOW + 8 * HOUR, NOW + 10 * HOUR)], NOW)
    expect(line).toBe('Next: Sprint in 8 h (Jul 7).')
  })
  it('says so plainly when nothing is coming up', () => {
    expect(contestsLine([ev('Old', NOW - 2 * DAY, NOW - DAY)], NOW)).toBe('No contests coming up.')
  })
})
