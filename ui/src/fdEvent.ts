/**
 * Field Day event-date helpers.
 *
 * ARRL Field Day: 4th full weekend of June, 1800 UTC Saturday.
 *   "Full weekend" = a Sat+Sun where Saturday is in June.
 *   The 4th such Saturday = the Saturday on or after June 22, but a Saturday
 *   that is the 4th Saturday of June (i.e. the Nth Saturday of the month where
 *   N = 4). ARRL defines it as "the 4th full weekend (Sat+Sun) of June", which
 *   resolves to the 4th Saturday that falls in June.
 *
 * Winter Field Day: last full weekend of January, 1600 UTC Saturday.
 *   = the last Saturday in January.
 */

export type FdKind = 'arrlfd' | 'wfd'

export interface FdEvent {
  kind: FdKind
  /** UTC start of the event (Saturday 1800 UTC for ARRL FD; 1600 UTC for WFD). */
  startUnix: number
  /** UTC end of the event (Sunday at the same clock hour). */
  endUnix: number
  /** Human label, e.g. "ARRL Field Day" or "Winter Field Day". */
  label: string
  /** Year the event is in. */
  year: number
}

/**
 * Return midnight UTC (00:00:00 UTC) for a given year/month(1-based)/day.
 * This is the canonical way to build a Date without timezone confusion.
 */
function utcDate(year: number, month: number, day: number): Date {
  return new Date(Date.UTC(year, month - 1, day))
}

/**
 * Find the Nth occurrence of a given weekday (0=Sun..6=Sat) in the given
 * year+month. Returns the day-of-month.
 */
function nthWeekdayOfMonth(year: number, month: number, weekday: number, n: number): number {
  // Find the first occurrence of `weekday` in that month.
  const first = utcDate(year, month, 1)
  const dow = first.getUTCDay()
  const firstOccurrence = ((weekday - dow + 7) % 7) + 1
  return firstOccurrence + (n - 1) * 7
}

/**
 * Find the last occurrence of a given weekday (0=Sun..6=Sat) in the given
 * year+month. Returns the day-of-month.
 */
function lastWeekdayOfMonth(year: number, month: number, weekday: number): number {
  // Days in month — easiest: day 0 of next month = last day of this month.
  const daysInMonth = new Date(Date.UTC(year, month, 0)).getUTCDate()
  const last = utcDate(year, month, daysInMonth)
  const dow = last.getUTCDay()
  const diff = (dow - weekday + 7) % 7
  return daysInMonth - diff
}

const SATURDAY = 6

/** ARRL Field Day Saturday for a given year (1800 UTC, Unix seconds). */
export function arrlFdSaturdayUnix(year: number): number {
  const day = nthWeekdayOfMonth(year, 6 /* June */, SATURDAY, 4)
  // 1800 UTC on that Saturday
  return Date.UTC(year, 5 /* 0-indexed */, day, 18, 0, 0) / 1000
}

/** Winter Field Day Saturday for a given year (1600 UTC, Unix seconds).
 * "Last FULL weekend of January" — BOTH days must land in January, which is
 * what WFDA actually schedules: 2025 = Jan 25–26 (not Sat 25? last Sat IS 25),
 * 2026 = Jan 24–25 (NOT Sat Jan 31, whose Sunday is Feb 1). A bare
 * last-Saturday rule got 2026 wrong by a week. */
export function wfdSaturdayUnix(year: number): number {
  let day = lastWeekdayOfMonth(year, 1 /* January */, SATURDAY)
  const daysInJan = 31
  if (day + 1 > daysInJan) {
    day -= 7 // the Sunday would spill into February — step back one weekend
  }
  // 1600 UTC on that Saturday
  return Date.UTC(year, 0 /* 0-indexed */, day, 16, 0, 0) / 1000
}

/**
 * Given a current date `now` and an event kind, return the NEXT upcoming (or
 * currently running) occurrence of that event.
 *
 * "Running" means we are between startUnix and endUnix (24 hours of event).
 * After the event ends, return the next year's event.
 */
export function fdNextEvent(now: Date, kind: FdKind): FdEvent {
  const nowUnix = Math.floor(now.getTime() / 1000)
  const year = now.getUTCFullYear()

  /** Build an FdEvent for a given year. */
  const build = (y: number): FdEvent => {
    if (kind === 'arrlfd') {
      const startUnix = arrlFdSaturdayUnix(y)
      return {
        kind,
        startUnix,
        // 24-hour event: ends Sunday at 1800 UTC
        endUnix: startUnix + 24 * 3600,
        label: 'ARRL Field Day',
        year: y,
      }
    } else {
      const startUnix = wfdSaturdayUnix(y)
      return {
        kind,
        startUnix,
        // 24-hour event: ends Sunday at 1600 UTC
        endUnix: startUnix + 24 * 3600,
        label: 'Winter Field Day',
        year: y,
      }
    }
  }

  // Try this year first; if its end has already passed, use next year.
  const thisYear = build(year)
  if (nowUnix < thisYear.endUnix) return thisYear

  // WFD: if we're past January of this year, next occurrence is January of next year.
  // ARRL FD: if we're past June of this year, next occurrence is June of next year.
  return build(year + 1)
}

/**
 * Format a countdown string like "starts in 18 days" or "in 2 hours".
 * Returns null when the event is currently running ("active").
 */
export function fdCountdownLabel(now: Date, event: FdEvent): string | null {
  const nowUnix = Math.floor(now.getTime() / 1000)
  if (nowUnix >= event.startUnix && nowUnix < event.endUnix) return null // active

  const secsUntil = event.startUnix - nowUnix
  if (secsUntil <= 0) return null

  const days = Math.floor(secsUntil / 86400)
  const hours = Math.floor((secsUntil % 86400) / 3600)

  if (days >= 2) return `starts in ${days} days`
  if (days === 1) return `starts tomorrow`
  if (hours >= 1) return `starts in ${hours}h`
  return 'starting soon'
}

/**
 * Format the event header subtitle, e.g.:
 *   "ARRL Field Day: Jun 28–29 · starts in 18 days"
 *   "Winter Field Day: Jan 24–25 · active"
 */
export function fdHeaderSubtitle(now: Date, event: FdEvent): string {
  const start = new Date(event.startUnix * 1000)
  const end = new Date(event.endUnix * 1000)

  const months = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']
  const sm = months[start.getUTCMonth()]
  const sd = start.getUTCDate()
  const em = months[end.getUTCMonth()]
  const ed = end.getUTCDate()

  const dateRange = sm === em
    ? `${sm} ${sd}–${ed}`
    : `${sm} ${sd}–${em} ${ed}`

  const countdown = fdCountdownLabel(now, event)
  const suffix = countdown ? ` · ${countdown}` : ' · active'

  return `${event.label}: ${dateRange}${suffix}`
}
