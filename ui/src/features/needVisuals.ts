import {
  Globe2,
  Compass,
  Layers,
  Radio,
  Grid3x3,
  MapPin,
  Tent,
  MailQuestion,
  TreePine,
  Mountain,
  Star,
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
  | 'state'
  | 'dxped'
  | 'confirm'
  | 'pota'
  | 'sota'
  | 'wanted'

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
  zone: { cls: 'need-zone', Icon: Compass, label: 'ZONE', title: 'New CQ zone (WAZ) — not yet worked on any band' },
  band: { cls: 'need-band', Icon: Layers, label: 'BAND', title: 'New band-slot for this entity' },
  mode: { cls: 'need-mode', Icon: Radio, label: 'MODE', title: 'New mode for this entity' },
  grid: { cls: 'need-grid', Icon: Grid3x3, label: 'GRID', title: 'New grid square on this band (VUCC is per band)' },
  state: { cls: 'need-state', Icon: MapPin, label: 'STATE', title: 'New US state on this band (5BWAS) — a hint from the grid; confirm from the log' },
  dxped: { cls: 'need-dxped', Icon: Tent, label: 'DXPED', title: 'Active DXpedition — limited-time window', iconOnly: true },
  confirm: { cls: 'need-confirm', Icon: MailQuestion, label: 'CONFIRM', title: 'Worked — needs a confirmation (QSL)' },
  pota: { cls: 'need-pota', Icon: TreePine, label: 'POTA', title: 'Live POTA activator', iconOnly: true },
  sota: { cls: 'need-sota', Icon: Mountain, label: 'SOTA', title: 'Live SOTA activator', iconOnly: true },
  wanted: { cls: 'need-wanted', Icon: Star, label: 'WANTED', title: 'On your wanted watch list' },
}

/** Canonical precedence (icon order left→right; also picks the row colour): the most
 * chase-worthy reason first. */
export const NEED_PRECEDENCE: NeedCat[] = [
  'wanted',
  'entity',
  'zone',
  'band',
  'mode',
  'grid',
  'state',
  'dxped',
  'confirm',
  'pota',
  'sota',
]

/** The need-chip vocabulary — the ONE source for chip text/class/tooltip per
 * NeedTag (this record used to be duplicated with drifting wording across
 * StationCard, OperateRoster, NeededPanel, and the Connect paneFormat).
 * `label` is the full board wording; `short` is for dense columns — the
 * surface picks, the words stay in one place. */
export const NEED_CHIP: Record<
  import('../types').NeedTag,
  { label: string; short: string; cls: string; title: string }
> = {
  NewEntity: {
    label: 'NEW ONE',
    short: 'NEW',
    cls: 'entity',
    title: 'All-time-new DXCC entity (ATNO)',
  },
  NewZone: { label: 'ZONE', short: 'ZONE', cls: 'zone', title: 'New CQ zone (WAZ) — not yet worked on any band' },
  NewBand: { label: 'BAND', short: 'BAND', cls: 'band', title: 'New band-slot for this entity' },
  NewMode: { label: 'MODE', short: 'MODE', cls: 'mode', title: 'New mode for this entity' },
  NewGrid: { label: 'GRID', short: 'GRID', cls: 'grid', title: 'New grid square on this band' },
  NewState: { label: 'STATE', short: 'ST', cls: 'state', title: 'New US state on this band — best-guess from the grid' },
  Confirm: {
    label: 'CONFIRM',
    short: 'CNF',
    cls: 'confirm',
    title: 'Worked — needs a confirmation',
  },
  Dxped: {
    label: 'DXPED',
    short: 'DXP',
    cls: 'dxped',
    title: 'Active announced DXpedition — a limited-time window',
  },
  Pota: {
    label: 'POTA',
    short: 'POTA',
    cls: 'pota',
    title: "Live POTA activator — the row's call is on a park right now",
  },
  Sota: {
    label: 'SOTA',
    short: 'SOTA',
    cls: 'sota',
    title: "Live SOTA activator — the row's call is on a summit right now",
  },
  Wanted: {
    label: 'WANTED',
    short: 'WANT',
    cls: 'wanted',
    title: 'On your wanted watch list',
  },
}
