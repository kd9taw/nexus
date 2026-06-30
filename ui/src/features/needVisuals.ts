import {
  Globe2,
  Compass,
  Layers,
  Radio,
  Grid3x3,
  Tent,
  MailQuestion,
  TreePine,
  Mountain,
  type LucideIcon,
} from 'lucide-react'

/** A reason a heard station is worth working, in one vocabulary shared by the Needed
 * panel and the band-activity decode feed so the two views read as one system. */
export type NeedCat =
  | 'entity'
  | 'zone'
  | 'band'
  | 'mode'
  | 'grid'
  | 'dxped'
  | 'confirm'
  | 'pota'
  | 'sota'

export interface NeedVisual {
  /** CSS class suffix — pairs with the `--need-*` palette (`.decode-row.need-*`,
   * `.need-chip.need-*`, `.np-row.need-*`). */
  cls: string
  Icon: LucideIcon
  /** Short text badge — the SAME vocabulary the Needed panel's chips use, so the decode
   * feed and the board read identically ("NEW ONE", "BAND", "POTA"…). */
  label: string
  title: string
  /** Icon-only categories that must NOT drive row colour (mirrors NeededPanel, where
   * dxped/pota/sota are appended and never `tags[0]`); the award tier keeps the colour. */
  iconOnly?: boolean
}

export const NEED_VISUALS: Record<NeedCat, NeedVisual> = {
  entity: { cls: 'need-entity', Icon: Globe2, label: 'NEW ONE', title: 'New DXCC entity — an all-time new one' },
  zone: { cls: 'need-zone', Icon: Compass, label: 'ZONE', title: 'New CQ zone (WAZ)' },
  band: { cls: 'need-band', Icon: Layers, label: 'BAND', title: 'New band-slot for this entity' },
  mode: { cls: 'need-mode', Icon: Radio, label: 'MODE', title: 'New mode for this entity' },
  grid: { cls: 'need-grid', Icon: Grid3x3, label: 'GRID', title: 'New grid square' },
  dxped: { cls: 'need-dxped', Icon: Tent, label: 'DXPED', title: 'Active DXpedition — limited-time window', iconOnly: true },
  confirm: { cls: 'need-confirm', Icon: MailQuestion, label: 'CONFIRM', title: 'Worked — needs a confirmation (QSL)' },
  pota: { cls: 'need-pota', Icon: TreePine, label: 'POTA', title: 'Live POTA activator', iconOnly: true },
  sota: { cls: 'need-sota', Icon: Mountain, label: 'SOTA', title: 'Live SOTA activator', iconOnly: true },
}

/** Canonical precedence (icon order left→right; also picks the row colour): the most
 * chase-worthy reason first. */
export const NEED_PRECEDENCE: NeedCat[] = [
  'entity',
  'zone',
  'band',
  'mode',
  'grid',
  'dxped',
  'confirm',
  'pota',
  'sota',
]
