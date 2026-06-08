import { useEffect, useState } from 'react'
import type {
  JourneyCollection,
  JourneyFeat,
  JourneyFirst,
  JourneyLadder,
  JourneySummary,
  JourneyTier,
} from '../types'
import { getJourney } from '../api'
import { StateBlock } from './StateBlock'

/**
 * Journey — the in-app, beginner-first achievement layer (separate from the
 * official Awards tracker). It turns the operator's own log into a living sense of
 * progress: a level/XP spine, auto-detected "firsts", tiered sub-award ladders that
 * climb toward the big awards, fill-the-map collections, novel ham feats, personal
 * bests, and an opt-in weekly streak. Pure read of `get_journey` — informational
 * feedback, never a coercive carrot.
 */
export function JourneyView() {
  const [j, setJourney] = useState<JourneySummary | null>(null)
  const [err, setErr] = useState<string | null>(null)

  useEffect(() => {
    let alive = true
    getJourney()
      .then((s) => alive && setJourney(s))
      .catch((e) => alive && setErr(e instanceof Error ? e.message : String(e)))
    return () => {
      alive = false
    }
  }, [])

  if (err)
    return (
      <main className="layout single">
        <StateBlock kind="error" title="Couldn't load your Journey" detail={err} />
      </main>
    )
  if (!j)
    return (
      <main className="layout single">
        <StateBlock kind="loading" title="Loading your Journey…" detail="Reading your log." />
      </main>
    )

  const xpPct = j.xpForLevel > 0 ? Math.min(100, (j.xpIntoLevel / j.xpForLevel) * 100) : 0
  const firstsDone = j.firsts.filter((f) => f.unlocked).length

  return (
    <main className="layout single journey-view">
      {/* Hero: level + XP + the single most-attainable next milestone (goal-gradient) */}
      <section className="jy-hero panel">
        <div className="jy-level">
          <div className="jy-level-badge" title={`${j.xp.toLocaleString()} XP earned`}>
            <span className="jy-level-num">{j.level}</span>
            <span className="jy-level-cap">level</span>
          </div>
          <div className="jy-level-bar-wrap">
            <div className="jy-level-top">
              <strong>Level {j.level}</strong>
              <span className="jy-xp">
                {j.xpIntoLevel.toLocaleString()} / {j.xpForLevel.toLocaleString()} XP to level{' '}
                {j.level + 1}
              </span>
            </div>
            <div className="jy-bar">
              <div className="jy-bar-fill" style={{ width: `${xpPct}%` }} />
            </div>
            <div className="jy-hero-meta">
              <span>{j.totalQsos.toLocaleString()} QSOs logged</span>
              {j.streak.enabled && j.streak.weeks > 0 && (
                <span className="jy-streak" title="Consecutive weeks with at least one contact">
                  {j.streak.weeks} week{j.streak.weeks === 1 ? '' : 's'} on the air
                  {j.streak.activeThisWeek ? '' : ' · this week pending'}
                </span>
              )}
            </div>
          </div>
        </div>
        {j.nextMilestone && (
          <div className="jy-next" title="Your most-attainable next milestone">
            <span className="jy-next-cap">Next milestone</span>
            <strong className="jy-next-title">{j.nextMilestone.title}</strong>
            <span className="jy-next-go">
              {j.nextMilestone.remaining} to go ({j.nextMilestone.current}/{j.nextMilestone.target})
            </span>
          </div>
        )}
      </section>

      {/* Firsts — the moments that kill the first-100-QSO motivational dead zone. */}
      <section className="jy-section">
        <div className="jy-section-head">
          <h2>Firsts</h2>
          <span className="jy-count">
            {firstsDone}/{j.firsts.length}
          </span>
        </div>
        <div className="jy-firsts">
          {j.firsts.map((f) => (
            <FirstChip key={f.id} first={f} />
          ))}
        </div>
      </section>

      {/* Ladders — tiered sub-awards climbing toward the big official awards. */}
      <section className="jy-section">
        <div className="jy-section-head">
          <h2>Climb toward the awards</h2>
          <span className="jy-section-note">
            Sub-award ladders — the official awards are the capstones in the Awards tab.
          </span>
        </div>
        <div className="jy-ladders">
          {j.ladders.map((l) => (
            <LadderCard key={l.id} ladder={l} />
          ))}
        </div>
      </section>

      {/* Collections — fill-the-map boards. */}
      <section className="jy-section">
        <div className="jy-section-head">
          <h2>Collections</h2>
        </div>
        <div className="jy-collections">
          {j.collections.map((c) => (
            <CollectionCard key={c.id} collection={c} />
          ))}
        </div>
      </section>

      {/* Feats — novel, ham-native accomplishments. */}
      <section className="jy-section">
        <div className="jy-section-head">
          <h2>Feats</h2>
        </div>
        <div className="jy-feats">
          {j.feats.map((f) => (
            <FeatCard key={f.id} feat={f} />
          ))}
        </div>
      </section>

      {/* Personal bests — your own station records. */}
      {j.bests.length > 0 && (
        <section className="jy-section">
          <div className="jy-section-head">
            <h2>Personal bests</h2>
          </div>
          <div className="jy-bests">
            {j.bests.map((b) => (
              <div className="jy-best" key={b.id}>
                <span className="jy-best-k">{b.title}</span>
                <span className="jy-best-v">{b.value}</span>
                {b.detail && <span className="jy-best-d">{b.detail}</span>}
              </div>
            ))}
          </div>
        </section>
      )}
    </main>
  )
}

function FirstChip({ first }: { first: JourneyFirst }) {
  const title = first.unlocked
    ? `${first.meaning}${first.heritage ? `\n\n${first.heritage}` : ''}`
    : `Locked — ${first.meaning}`
  return (
    <div className={`jy-first${first.unlocked ? ' done' : ''}`} title={title}>
      <span className="jy-first-mark">{first.unlocked ? '✦' : '○'}</span>
      <span className="jy-first-body">
        <span className="jy-first-title">{first.title}</span>
        {first.unlocked && first.detail && <span className="jy-first-detail">{first.detail}</span>}
      </span>
    </div>
  )
}

function LadderCard({ ladder }: { ladder: JourneyLadder }) {
  const pct = ladder.max > 0 ? Math.min(100, (ladder.worked / ladder.max) * 100) : 0
  const cpct = ladder.max > 0 ? Math.min(100, (ladder.confirmed / ladder.max) * 100) : 0
  const done = ladder.worked >= ladder.max
  return (
    <div className={`jy-ladder${done ? ' complete' : ''}`} title={ladder.heritage}>
      <div className="jy-ladder-head">
        <strong>{ladder.title}</strong>
        <span className="jy-ladder-count">
          {ladder.worked}
          <span className="jy-dim"> worked</span> · {ladder.confirmed}
          <span className="jy-dim"> confirmed</span> / {ladder.max}
        </span>
      </div>
      <p className="jy-ladder-meaning">{ladder.meaning}</p>
      <div className="jy-ladder-track">
        {/* worked (outer) + confirmed (inner) fills */}
        <div className="jy-bar jy-ladder-bar">
          <div className="jy-bar-fill worked" style={{ width: `${pct}%` }} />
          <div className="jy-bar-fill confirmed" style={{ width: `${cpct}%` }} />
        </div>
        {/* rung ticks */}
        {ladder.rungs.map((r) => (
          <span
            key={r.label}
            className={`jy-rung${ladder.worked >= r.target ? ' hit' : ''} jy-tier-${r.tier}`}
            style={{ left: `${Math.min(100, (r.target / ladder.max) * 100)}%` }}
            title={`${r.label} — ${r.target}`}
          />
        ))}
      </div>
      {ladder.nextRung ? (
        <div className="jy-ladder-next">
          <span className={`jy-tier-pill jy-tier-${ladder.nextRung.tier}`}>
            {ladder.nextRung.label}
          </span>
          <span className="jy-ladder-go">
            {ladder.nextRung.target - ladder.worked} to go
          </span>
        </div>
      ) : (
        <div className="jy-ladder-next">
          <span className="jy-tier-pill jy-tier-platinum">Complete ★</span>
        </div>
      )}
    </div>
  )
}

function CollectionCard({ collection }: { collection: JourneyCollection }) {
  // Few cells (continents / band×mode) get labels; large sets render as a tight grid.
  const labelled = collection.cells.length <= 16
  const pct = collection.total > 0 ? Math.round((collection.worked / collection.total) * 100) : 0
  return (
    <div className="jy-collection">
      <div className="jy-collection-head">
        <strong>{collection.title}</strong>
        <span className="jy-collection-count">
          {collection.worked}/{collection.total} · {pct}%
        </span>
      </div>
      <p className="jy-collection-meaning">{collection.meaning}</p>
      <div className={`jy-cells${labelled ? ' labelled' : ''}`}>
        {collection.cells.map((c) => (
          <span
            key={c.key}
            className={`jy-cell${c.worked ? ' worked' : ''}${c.confirmed ? ' confirmed' : ''}`}
            title={`${c.label}${c.confirmed ? ' — confirmed' : c.worked ? ' — worked' : ' — needed'}`}
          >
            {labelled ? c.label : ''}
          </span>
        ))}
      </div>
    </div>
  )
}

const TIER_LABEL: Record<JourneyTier, string> = {
  bronze: 'Bronze',
  silver: 'Silver',
  gold: 'Gold',
  platinum: 'Platinum',
  legendary: 'Legendary',
}

function FeatCard({ feat }: { feat: JourneyFeat }) {
  const pct = feat.target > 0 ? Math.min(100, (feat.current / feat.target) * 100) : 0
  const fmt = (n: number) => (Number.isInteger(n) ? String(n) : n.toFixed(0))
  return (
    <div
      className={`jy-feat${feat.unlocked ? ' done' : ''}${feat.gated ? ' gated' : ''} jy-tier-${feat.tier}`}
      title={feat.heritage}
    >
      <div className="jy-feat-head">
        <span className="jy-feat-mark">{feat.unlocked ? '★' : feat.gated ? '🔒' : '○'}</span>
        <strong>{feat.title}</strong>
        <span className={`jy-tier-pill jy-tier-${feat.tier}`}>{TIER_LABEL[feat.tier]}</span>
      </div>
      <p className="jy-feat-meaning">{feat.meaning}</p>
      {feat.gated ? (
        <p className="jy-feat-gate">{feat.gateHint}</p>
      ) : (
        <>
          <div className="jy-bar">
            <div className="jy-bar-fill" style={{ width: `${pct}%` }} />
          </div>
          <div className="jy-feat-foot">
            <span>
              {fmt(feat.current)} / {fmt(feat.target)} {feat.unit}
            </span>
            {feat.detail && <span className="jy-dim">{feat.detail}</span>}
          </div>
        </>
      )}
    </div>
  )
}
