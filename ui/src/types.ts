// Shared JSON DTO contract (camelCase).
// These shapes MUST match the Rust app-logic layer exactly so the web UI
// interoperates with the Tempo core over Tauri `invoke`.

export type Presence = 'active' | 'idle' | 'stale'

export type Tier = 'FT1' | 'DX1' | 'FT8' | 'FT4'

// ---- Propagation & opening intelligence (matches the `propagation` crate) ----
export type ActivityTier = 'Active' | 'Moderate' | 'Quiet' | 'Closed'
export type Confidence = 'Strong' | 'Likely' | 'Marginal'
export type NeedKind = 'Atno' | 'NewBand' | 'NewMode' | 'Confirm' | 'Satisfied'
export type WorkStatus = 'WorkNow' | 'OpeningPredicted' | 'NotOpen'

export interface RegionReport {
  region: string
  octant: string
  bearingDeg: number
  stations: number
  bidirectional: boolean
}
export interface BandReport {
  band: string
  tier: ActivityTier
  score: number
  nHearMe: number
  nIHear: number
  bestRegion: RegionReport | null
  confidence: Confidence
  reason: string
}
export interface PropAdvisory {
  headline: string
  bands: BandReport[]
  banners: string[]
}
export interface OpeningView {
  band: string
  mode: string
  octant: string
  bearingDeg: number
  maxKm: number
  probability: number
  stations: number
  confidence: string
  note: string
}
export interface WorkableCard {
  call: string
  entity: string
  need: NeedKind
  band: string
  bearingDeg: number
  octant: string
  distanceKm: number
  status: WorkStatus
  /** Modelled contact-likelihood word (Closed/Marginal/Fair/Good/Excellent). */
  likelihood: string
  /** Likelihood score 0..1 (model, possibly upgraded by live evidence). */
  likelihoodScore: number
  /** Live PSK Reporter spots confirm this band toward the DX region. */
  liveConfirmed: boolean
  howToCall: string
  windowHint: string
  priority: number
}
/** One band's outlook on a path: best workability + window + per-UTC-hour grid. */
export interface BandOutlook {
  band: string
  workability: string
  score: number
  window: string
  /** Per-UTC-hour likelihood (24 values, hour 0..23) — the heatmap row. */
  hourly: number[]
}
/** A forward-calendar entry — an announced DXpedition to plan for. */
export interface CalendarEntry {
  call: string
  entity: string
  region: string
  startUnix: number
  endUnix: number
  bands: string[]
  modes: string[]
  octant: string
  bearingDeg: number
  distanceKm: number
  /** Best-band outlooks (modelled daily windows + heatmap rows). */
  outlook: BandOutlook[]
  /** One-line headline, e.g. "20m Good 1400–1700Z". */
  best: string
}
export interface DxpedDashboard {
  workableNow: WorkableCard[]
  active: string[]
  upcoming: CalendarEntry[]
}
export interface SpaceWxView {
  sfi: number
  kp: number
  aIndex: number
  xrayClass: string
  flare: boolean
}
export interface PropagationSnapshot {
  advisory: PropAdvisory
  openings: OpeningView[]
  dxpeditions: DxpedDashboard
  spaceWx: SpaceWxView
  /** Provenance: 'live' (fresh), 'cached' (stale last-good), or 'demo'. */
  source: 'live' | 'cached' | 'demo'
  /** When this data was produced (Unix seconds, UTC). */
  asOf: number
}

/** Top-level operating mode reflected in the snapshot. */
export type OpMode = 'chat' | 'qso' | 'fieldDay'

/** How PTT (transmit keying) is asserted. */
export type PttMethod = 'cat' | 'rts' | 'dtr' | 'vox'

/** SSB sideband / phone mode used for a channel. */
export type RadioMode = 'USB' | 'FM'

/** A preset entry in the band plan (one tap to QSY there). */
export interface BandChannel {
  band: string
  group: 'HF' | 'VHF' | 'UHF'
  dialMhz: number
  mode: RadioMode
  label: string
  note: string
}

/** Audio input + output device names discovered on the host. */
export interface AudioDevices {
  input: string[]
  output: string[]
}

/**
 * Mode-change request variants accepted by `set_mode`. The "-run" / "-monitor"
 * / "-sp" suffixes pick the operator role within QSO / Field Day modes.
 */
export type ModeRequest =
  | 'chat'
  | 'qso-run'
  | 'qso-monitor'
  | 'fieldday-run'
  | 'fieldday-sp'

export interface Station {
  call: string
  grid: string | null
  snr: number
  lastHeardSlot: number
  heardCount: number
  presence: Presence
  /** True if this callsign has been worked (logged) before. */
  worked: boolean
}

export interface ChatMessage {
  from: string | null
  to: string | null
  text: string
  slot: number
  directedToMe: boolean
  outbound: boolean
  snr: number | null
  freqHz: number | null
  dtSec: number | null
  tier: Tier | null
}

export interface Conversation {
  peer: string
  messages: ChatMessage[]
}

export interface LinkState {
  tier: Tier
  snrDb: number
  dtSec: number
  freqHz: number
  rv: number
  state: string
  quality: number
}

export interface RadioStatus {
  dialMhz: number
  band: string
  sideband: string
  transmitting: boolean
  slot: number
  nextSlotMs: number
  timeSyncOk: boolean
  /** Incoming audio level, 0–1 (drives the RX meter; ~1.0 = clipping). */
  rxLevel: number
  /** Whether transmit is enabled (Monitor on). Off = muted/listening only. */
  txEnabled: boolean
  /** Whether a tune carrier is currently keyed. */
  tuning: boolean
  /** True if the TX watchdog has auto-halted transmit (needs a re-enable). */
  txWatchdog: boolean
  /** Rig/CAT health: null/undefined = N/A (VOX); true = connected; false = failing. */
  catOk?: boolean | null
  /** Human-readable rig/CAT status (read frequency, or a specific error). */
  catDetail?: string
  /** Set when the sound card failed to open (explains a blank waterfall). */
  audioError?: string | null
  /** Transmit on even/"1st" slots (true) or odd/"2nd" (false). */
  txEven: boolean
  /** Receive audio offset (Hz) — the green waterfall marker. */
  rxOffsetHz: number
  /** Transmit audio offset (Hz) — the red waterfall marker. */
  txOffsetHz: number
  /** Keep the TX offset fixed when RX changes ("Hold Tx Freq"). */
  holdTxFreq: boolean
  /** Real PC-clock-vs-UTC offset in ms (positive = fast), or null if offline/disabled. */
  clockOffsetMs?: number | null
  /** Where decodes come from: the native engine or a WSJT-X/JTDX/MSHV companion. */
  source: SourceKind
  /** Human-readable source label, e.g. "Native (FT8)" or "WSJT-X UDP". */
  sourceLabel: string
}

/** Signal source: decode locally ('native') or ride an upstream WSJT-X over UDP. */
export type SourceKind = 'native' | 'companion'

/** Result of a "Test CAT" probe: reachable + a human-readable detail line. */
export interface CatTestResult {
  ok: boolean
  detail: string
}

export interface Spectrum {
  // one waterfall row, 0..1 magnitudes, ~120 bins
  row: number[]
}

/** A single decoded signal in the most-recent RX slot (WSJT-X style row). */
export interface DecodeRow {
  from: string | null
  snr: number
  dtSec: number
  freqHz: number
  message: string
  isCq: boolean
  directedToMe: boolean
  worked: boolean
  tier: Tier
  /** IR-HARQ redundancy versions combined to recover this decode: 0 = decoded
   * from the initial transmission alone; 1/2 = recovered by joint-combining that
   * many retransmissions; -1 = not applicable. Used to badge HARQ rescues. */
  rv: number
}

/** A logged contact in the general ADIF logbook (separate from Field Day). */
export interface LoggedQso {
  call: string
  grid: string | null
  /** US state (ADIF STATE, 2-letter) for WAS, when known. */
  state?: string | null
  band: string
  freqMhz: number
  mode: string
  rstSent: number | null
  rstRcvd: number | null
  /** Contact time, seconds since the Unix epoch (UTC). */
  whenUnix: number
  /** Confirmed via ANY channel (LoTW / eQSL / paper QSL). */
  confirmed: boolean
  /** Award-eligible confirmation (LoTW or paper only — eQSL excluded). */
  awardConfirmed: boolean
  /** Awards credit granted by ARRL (normalized ADIF codes, e.g. "DXCC"). */
  creditGranted?: string[]
  /** Awards credit applied/submitted but not yet granted. */
  creditSubmitted?: string[]
}

/** A confirmation in a synced report with no matching logged QSO (diagnostic). */
export interface LotwOrphan {
  call: string
  band: string
  mode: string
  whenUnix: number
  reason: string
}

/** Result of reconciling a LoTW (or any ADIF) confirmation report into the log. */
export interface LotwSyncResult {
  matched: number
  newlyConfirmed: number
  newlyCredited: number
  newlySubmitted: number
  orphans: LotwOrphan[]
}

/** Per-band DXCC entity progress (worked vs confirmed). */
export interface BandAward {
  band: string
  worked: number
  confirmed: number
}

/** Per-mode DXCC entity progress (CW / Phone / Digital — separate awards). */
export interface ModeAward {
  mode: string
  worked: number
  confirmed: number
}

/** A worked-but-unconfirmed DXCC entity — the "new one" chase. */
export interface EntityNeed {
  entity: string
  /** Bands the entity is worked-but-unconfirmed on. */
  bands: string[]
}

/** A gamification milestone (unlocked, or locked-with-progress). */
export interface Achievement {
  /** Stable id (e.g. "dxcc-100") — the key the UI tracks "seen" by. */
  id: string
  title: string
  detail: string
  /** Grouping label, e.g. QSOs / DXCC / DXpeditions / Challenge / WAZ / WAS. */
  category: string
  unlocked: boolean
  /** Progress toward `target` (the live stat). */
  current: number
  target: number
  /** Celebrate with a toast when newly unlocked (a big moment). */
  critical: boolean
}

/** DXCC-first award progress, computed from the logbook (cty.dat-resolved). */
/** Why a heard station is worth working (need-aware spotting). */
export type NeedTag = 'NewEntity' | 'NewZone' | 'NewBand' | 'NewMode' | 'Confirm'

/** A scored need opportunity for a station heard right now. */
export interface NeedAlert {
  call: string
  entity: string
  band: string
  zone: number
  tags: NeedTag[]
  priority: number
  headline: string
}

/** Worked All States progress (50 US states; LoTW/paper confirmed). */
export interface WasProgress {
  worked: number
  confirmed: number
  /** States still to confirm (postal codes, sorted) — the WAS chase. */
  needed: string[]
  /** 5-Band WAS: states worked / confirmed on all of 80/40/20/15/10m. */
  fiveBandWorked: number
  fiveBandConfirmed: number
}

/** DXCC Honor Roll standing — current-entity, confirmed. (ARRL: confirmed ≥
 * currentTotal − 9 = Honor Roll; all current entities = #1 Honor Roll.) */
export interface HonorRollProgress {
  /** Current DXCC entities (denominator) — derived from cty.dat (non-WAE). */
  currentTotal: number
  /** Confirmed current DXCC entities (numerator). */
  confirmed: number
  /** Entry threshold = currentTotal − 9. */
  threshold: number
  /** True once confirmed ≥ threshold. */
  achieved: boolean
  /** Confirmed entities still needed to reach Honor Roll entry (0 if achieved). */
  needed: number
  /** True once every current entity is confirmed (#1 Honor Roll). */
  numberOne: boolean
  /** Confirmed entities still needed for #1 Honor Roll (0 if achieved). */
  numberOneNeeded: number
}

export interface AwardSummary {
  qsos: number
  confirmedQsos: number
  /** Distinct DXCC entities worked / confirmed (100 confirmed = basic DXCC). */
  dxccWorked: number
  dxccConfirmed: number
  /** Distinct DXCC entities with ARRL credit granted (official standing). */
  dxccCredited: number
  /** Confirmed-but-not-credited entities (confirmed − credited) — ready to submit. */
  readyToSubmit: number
  /** Entity×band "DXCC Challenge" slots worked / confirmed. */
  slotsWorked: number
  slotsConfirmed: number
  /** Per-band entity progress, band-ordered (160m → 2m). */
  bands: BandAward[]
  /** Per-mode DXCC progress (CW / Phone / Digital). */
  modes: ModeAward[]
  /** Worked-but-unconfirmed entities (new-DXCC-entity chase), most-bands first. */
  needed: EntityNeed[]
  /** DXCC-Challenge chase: already-confirmed entities still needing band slots
   * (worked-but-unconfirmed), most-bands first. */
  slotNeeded: EntityNeed[]
  /** Gamification milestones (unlocked + locked-with-progress). */
  achievements: Achievement[]
  /** 5-Band DXCC: entities worked / confirmed on all of 80/40/20/15/10m. */
  fiveBandWorked: number
  fiveBandConfirmed: number
  /** Worked All Zones (CQ WAZ): distinct CQ zones worked / confirmed, out of 40. */
  wazWorked: number
  wazConfirmed: number
  /** DXCC Honor Roll standing (current-entity, confirmed). */
  honorRoll: HonorRollProgress
  /** Worked All States (50 US states) + 5-Band WAS. */
  was: WasProgress
  /** WORK chase: entities worked on most award bands but missing a few — the
   * listed bands are ones to WORK (a new contact). Closest-to-complete first. */
  bandTargets: EntityNeed[]
}

/** Result of importing an external ADIF logbook (deduped merge). */
export interface ImportStats {
  added: number
  skipped: number
  total: number
}

/** Sequencer status for a 1:1 QSO. */
export interface QsoStatus {
  state: string
  dxcall: string | null
  rxReport: number | null
  running: boolean
}

/**
 * Coordinated-QSY ("move together") status — present only while the opt-in
 * feature is enabled. Announced in the clear: NOT private, NOT encrypted.
 */
export interface QsyStatus {
  enabled: boolean
  paused: boolean
  /** "initiator" (announces moves) | "follower" (auto-follows) | "idle" (no partner). */
  role: string
  partner: string | null
  /** Home channel token (where the conversation started). */
  home: string | null
  /** Channel token we're currently on. */
  current: string | null
  /** Next scheduled move's target channel token (HOME = return home), if any. */
  nextChannel: string | null
  /** Absolute UTC slot the next move executes on, if scheduled. */
  nextSlot: number | null
  /** True after a "lost sync → home" fall-back fired. */
  lostSync: boolean
}

/** A single logged Field Day contact. */
export interface FieldDayQso {
  call: string
  class: string
  section: string
  band: string
}

/** Field Day operating + scoring status. */
export interface FieldDayStatus {
  myClass: string
  mySection: string
  running: boolean
  state: string
  qsoCount: number
  sections: number
  points: number
  log: FieldDayQso[]
}

/** Persistent operator + radio settings. */
export interface Settings {
  mycall: string
  mygrid: string
  band: string
  dialMhz: number
  sideband: string
  fdClass: string
  fdSection: string
  // --- rig control ---
  /** PTT keying method: CAT (rigctld) / serial RTS / serial DTR / VOX. */
  pttMethod: string
  /** Hamlib rig model number (0 = none / not selected). */
  rigModel: number
  /** Human-readable rig model name, paired with rigModel. */
  rigModelName: string
  /** Serial/COM port for rig control (e.g. "COM3" / "/dev/ttyUSB0"). */
  serialPort: string
  /** Serial baud rate. */
  baud: number
  /** TCP port that rigctld listens on / Tempo launches it with. */
  rigctldPort: number
  // --- audio ---
  /** Input (RX) audio device name. "" = system default. */
  audioIn: string
  /** Output (TX) audio device name. "" = system default. */
  audioOut: string
  /** Transmit drive level, 0–1 (default 0.9). */
  txLevel: number
  /** TX watchdog timeout in minutes (default 6, 0 = disabled). */
  txWatchdogMin: number
  // --- timing & tuning ---
  /** Transmit on even/"1st" slots (true) or odd/"2nd" (false). */
  txEven: boolean
  /** Receive audio offset (Hz). */
  rxOffsetHz: number
  /** Transmit audio offset (Hz). */
  txOffsetHz: number
  /** Keep TX offset fixed when RX changes ("Hold Tx Freq"). */
  holdTxFreq: boolean
  /** Periodically check the PC clock against an NTP server (default on). */
  clockCheck: boolean
  // --- network integrations ---
  /** Expose the WSJT-X-compatible UDP API (for loggers etc.). */
  wsjtxUdp: boolean
  /** Address:port for the WSJT-X UDP API. */
  wsjtxUdpAddr: string
  /** Upload spots to PSK Reporter's global map. */
  pskreporter: boolean
  /** Connect to a DX cluster / RBN for need-aware spots (opt-in; needs restart). */
  clusterEnabled?: boolean
  /**
   * Periodically call CQ to announce presence. OFF = passive (hunt & pounce):
   * Tempo only transmits when the operator acts. This is the ONLY auto-TX path.
   */
  beacon: boolean
  /**
   * IR-HARQ: combine RV1/RV2 retransmissions at the receiver and escalate the
   * redundancy version on unacknowledged QSO transmissions. ON by default; off
   * forces RV0-only (each frame decoded independently).
   */
  harqEnabled: boolean
  // --- logbook & alerts ---
  /** Automatically log completed QSOs to the ADIF logbook. */
  autoLog: boolean
  // --- coordinated QSY ("move together") — separate, opt-in, off by default ---
  /** Master opt-in for coordinated QSY (announced-in-the-clear roaming). */
  qsyEnabled: boolean
  /** Band-plan channel tokens the initiator round-robins through when hopping. */
  qsySet: string[]
  /** Announce cadence: the initiator hops every this-many of its TX overs. */
  qsyCadence: number
  /** Alert (beep + flash) when a decode is directed at my callsign. */
  alertMyCall: boolean
  /** Alert when any station is calling CQ. */
  alertCq: boolean
  /** Alert when a station not heard before this session appears. */
  alertNew: boolean
  /** Editable quick-reply macros, per context. */
  macros: {
    chat: string[]
    qso: string[]
    band: string[]
  }
}

export interface AppSnapshot {
  mycall: string
  mygrid: string
  mode: OpMode
  radio: RadioStatus
  link: LinkState
  stations: Station[]
  conversations: Conversation[]
  activePeer: string | null
  qso: QsoStatus | null
  fieldDay: FieldDayStatus | null
  /** The most-recent RX slot's decoded signals (drives the live decode feed). */
  recentDecodes: DecodeRow[]
  /** Coordinated-QSY status — present only while the opt-in feature is enabled. */
  qsy?: QsyStatus | null
  /** Session count of IR-HARQ rescues (decodes recovered by combining
   * retransmissions). Drives the HARQ stats readout. */
  harqRescues: number
}
