// NCDXF/IARU beacon monitor (B2 pane Expert view). Pure clock schedule (which beacon is
// on each band now) + a "heard ✓" badge from the spots we already ingest. Self-ticks once
// a second so the 10 s slot + countdown stay live between the 30 s propagation polls.
import { useEffect, useState } from 'react'
import { beaconsNow, beaconHeard } from '../../features/beacons'
import type { MapSpot } from '../../types'

function ago(secs: number): string {
  return secs < 60 ? `${Math.round(secs)}s` : `${Math.round(secs / 60)}m`
}

export function BeaconMonitor({ spots, expert }: { spots: MapSpot[] | null; expert: boolean }) {
  const [, setTick] = useState(0)
  useEffect(() => {
    const id = window.setInterval(() => setTick((t) => t + 1), 1000)
    return () => window.clearInterval(id)
  }, [])
  const slots = beaconsNow(Date.now() / 1000)
  return (
    <ul className="bcn-list">
      {slots.map((s) => {
        const heard = beaconHeard(s.call, spots ?? undefined)
        return (
          <li key={s.band} className={`bcn-row${heard ? ' is-heard' : ''}`}>
            <span className="bcn-band">{s.band}</span>
            <span className="bcn-call" title={`${s.qth} · ${s.freqMhz} MHz`}>
              {s.call}
            </span>
            {expert && <span className="bcn-qth">{s.qth}</span>}
            <span className="bcn-bar" aria-hidden="true">
              <span className="bcn-fill" style={{ width: `${(s.secsIntoSlot + 1) * 10}%` }} />
            </span>
            <span className="bcn-heard">{heard ? `✓ ${ago(heard.ageSecs)}` : '—'}</span>
          </li>
        )
      })}
    </ul>
  )
}
