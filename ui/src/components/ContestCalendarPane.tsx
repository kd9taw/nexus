// The Contests pane — upcoming HF/VHF contests from the WA7BNM calendar
// (get_contests → parse_contest_rss). Self-fetching like SatPassesPane, but the
// concrete loader is injected so this file stays decoupled from api.ts (which is
// wired separately). Rows are grouped now / soon / this week / later; already-
// ended contests drop out. Honesty: null data → the pane renders nothing and
// PaneFrame falls back to the Basic hint (never a fabricated empty schedule).
import { useEffect, useState } from 'react'

/** One upcoming contest (mirrors the Rust `ContestEvent`, camelCase over the wire). */
export interface ContestEvent {
  name: string
  startUnix: number
  endUnix: number
  url?: string | null
}

export type ContestBucket = 'now' | 'soon' | 'week' | 'later'

const HOUR = 3600
const DAY = 86_400
const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']

/** Which "when" group a contest falls in, relative to `now` (unix seconds):
 *  now = currently running; soon = starts within 24 h; week = within 7 days; later = beyond. */
export function contestBucket(ev: ContestEvent, now: number): ContestBucket {
  if (ev.startUnix <= now && now < ev.endUnix) return 'now'
  const until = ev.startUnix - now
  if (until <= HOUR * 24) return 'soon'
  if (until <= DAY * 7) return 'week'
  return 'later'
}

/** Upcoming = not yet ended, soonest first. */
export function upcomingContests(list: ContestEvent[], now: number): ContestEvent[] {
  return list.filter((e) => e.endUnix > now).sort((a, b) => a.startUnix - b.startUnix)
}

function stamp(unix: number): { date: string; hhmm: string } {
  const d = new Date(unix * 1000)
  return {
    date: `${MONTHS[d.getUTCMonth()]} ${d.getUTCDate()}`,
    hhmm: `${String(d.getUTCHours()).padStart(2, '0')}${String(d.getUTCMinutes()).padStart(2, '0')}Z`,
  }
}

/** "Jul 11 1200Z → Jul 12 1200Z", collapsing the end date when it shares the start's day. */
export function formatRange(ev: ContestEvent): string {
  const s = stamp(ev.startUnix)
  const e = stamp(ev.endUnix)
  return s.date === e.date
    ? `${s.date} ${s.hhmm} → ${e.hhmm}`
    : `${s.date} ${s.hhmm} → ${e.date} ${e.hhmm}`
}

function relStart(ev: ContestEvent, now: number): string {
  const until = ev.startUnix - now
  if (until <= 0) return 'now'
  const h = Math.round(until / HOUR)
  return h < 24 ? `in ${h} h` : `in ${Math.round(until / DAY)} d`
}

/** One plain sentence summarizing the schedule (exported for reuse + tests). */
export function contestsLine(list: ContestEvent[] | null, now: number): string {
  if (!list) return 'Contest schedule loads once online.'
  const up = upcomingContests(list, now)
  if (up.length === 0) return 'No contests coming up.'
  const live = up.find((e) => contestBucket(e, now) === 'now')
  if (live) return `On now: ${live.name} (until ${stamp(live.endUnix).hhmm}).`
  const next = up[0]
  return `Next: ${next.name} ${relStart(next, now)} (${stamp(next.startUnix).date}).`
}

const GROUPS: { bucket: ContestBucket; label: string }[] = [
  { bucket: 'now', label: 'On the air now' },
  { bucket: 'soon', label: 'Starting soon' },
  { bucket: 'week', label: 'This week' },
  { bucket: 'later', label: 'Later' },
]

export function ContestCalendarPane({
  load,
  expert,
}: {
  /** Injected fetcher (registry passes the wired `getContests`). Null = no data yet. */
  load: () => Promise<ContestEvent[] | null>
  expert: boolean
}) {
  const [contests, setContests] = useState<ContestEvent[] | null>(null)
  useEffect(() => {
    let live = true
    const run = () =>
      load()
        .then((c) => live && setContests(c))
        .catch(() => {})
    run()
    // The calendar changes slowly (daily) — a lazy 15-min refresh is plenty.
    const id = window.setInterval(run, 15 * 60_000)
    return () => {
      live = false
      window.clearInterval(id)
    }
  }, [load])

  if (!contests) return null // → PaneFrame's honest Basic hint
  const now = Date.now() / 1000
  const up = upcomingContests(contests, now)
  if (up.length === 0) return null

  const shown = up.slice(0, expert ? 20 : 8)
  const byBucket = new Map<ContestBucket, ContestEvent[]>()
  for (const ev of shown) {
    const b = contestBucket(ev, now)
    const arr = byBucket.get(b) ?? []
    arr.push(ev)
    byBucket.set(b, arr)
  }

  return (
    <section className="contest-pane panel">
      {GROUPS.map(({ bucket, label }) => {
        const rows = byBucket.get(bucket)
        if (!rows || rows.length === 0) return null
        return (
          <div key={bucket} className={`cc-group cc-${bucket}`}>
            <h4 className="cc-group-label">{label}</h4>
            <ul className="cc-list">
              {rows.map((ev) => (
                <li key={`${ev.name}-${ev.startUnix}`} className="cc-row">
                  <span className="cc-name">
                    {ev.url ? (
                      <a href={ev.url} target="_blank" rel="noreferrer" title="Rules & details (WA7BNM)">
                        {ev.name}
                      </a>
                    ) : (
                      ev.name
                    )}
                  </span>
                  <span className="cc-when">{formatRange(ev)}</span>
                </li>
              ))}
            </ul>
          </div>
        )
      })}
    </section>
  )
}
