// The grid-rarity gem: ◆ (rare) / ◆◆ (ultra-rare, water-only) beside a call or
// grid wherever one is shown. Renders nothing for common/uncommon — the boards
// must not become confetti. The tooltip explains WHY it's rare (rarityMeta).
import { rarityMeta } from '../propViz'
import type { GridRarity } from '../types'

export function RarityGem({ rarity }: { rarity?: GridRarity | null }) {
  const m = rarityMeta(rarity)
  if (!m) return null
  return (
    <span className={`rarity-gem ${m.cls}`} title={m.title}>
      {m.glyph}
    </span>
  )
}
