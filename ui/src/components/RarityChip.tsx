// The grid-rarity marker. ULTRA-rare (open water — only rovers, maritime mobiles,
// or DXpeditions can activate it) graduates from the tiny ◆◆ gem to a loud, glowing
// "💎 ULTRA" pill so it's unmistakable at a glance and echoes the "💎 ULTRA-RARE"
// alert. RARE stays the quiet ◆ gem — the boards must not become confetti. Renders
// nothing for common/uncommon. Drop-in replacement for RarityGem.
import { rarityMeta } from '../propViz'
import type { GridRarity } from '../types'

export function RarityChip({ rarity }: { rarity?: GridRarity | null }) {
  const m = rarityMeta(rarity)
  if (!m) return null
  if (m.cls === 'ultra') {
    return (
      <span className="rarity-chip ultra" title={m.title}>
        💎 {m.label}
      </span>
    )
  }
  // Rare (and any other non-null tier) stays the quiet gem.
  return (
    <span className={`rarity-gem ${m.cls}`} title={m.title}>
      {m.glyph}
    </span>
  )
}
