// The single object handed to every Connect pane — built once in ConnectView from
// already-lifted state (no new computation). Field set is exactly what the B1 panes
// consume, plus a few forward-compat fields for B2/B3.
import type { Theme } from '../../useTheme'
import type { MapIntent } from '../MapView'
import type {
  AlertView,
  BandOutlook,
  GettingOut,
  MapSpot,
  MufStation,
  NeedAlert,
  NeedTag,
  NoaaScalesView,
  PathPrediction,
  PropagationSnapshot,
  Station,
  WorkableCard,
} from '../../types'

export interface PaneContext {
  // environment (B2/B3-ready; B1 panes mostly read prop/selection)
  myGrid: string
  theme: Theme
  intent: MapIntent
  expert: boolean
  // shared live state
  prop: PropagationSnapshot | null
  prov: { label: string; cls: string } | null
  needByCall: Map<string, NeedTag>
  needAlerts: NeedAlert[] // reserved for B2 best-band/needs pane
  // selection lifecycle
  selectedCall: string | null
  selStation: Station | null
  selSpot: MapSpot | null
  selDxped: WorkableCard | null
  selGrid: string | null
  // outlook (API-fetched in ConnectView)
  pathPred: PathPrediction | null
  bandOutlook: PathPrediction | null
  pathOpen: BandOutlook[]
  outlookOpen: BandOutlook[]
  // getting-out + band focus
  getout: GettingOut | null
  focusBand: string | null
  // B3 live external data (desktop-only; null/empty until the feeds answer)
  scales: NoaaScalesView | null
  alerts: AlertView[]
  muf: MufStation[]
  // callbacks
  onSelectCall: (call: string | null) => void
  onWorkSpot?: (t: { call: string; band: string; mode: string | null; freqMhz: number | null }) => void
  toggleFocusBand: (band: string) => void
}
