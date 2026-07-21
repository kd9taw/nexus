// Shared JSON DTO contract (camelCase).
// These shapes MUST match the Rust app-logic layer exactly so the web UI
// interoperates with the Tempo core over Tauri `invoke`.

export type Presence = 'active' | 'idle' | 'stale'

export type Tier = 'TempoFast' | 'TempoDeep' | 'FT8' | 'FT4'

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

/** One region's best band right now (inverse of BandReport.bestRegion) — the best-band
 *  recommender. Operator-anchored. */
export interface RegionBest {
  region: string
  octant: string
  bearingDeg: number
  band: string
  tier: ActivityTier
  modeled: BandModeled
  stations: number
  bidirectional: boolean
  score: number
}

/** One (region, band) cell of the operator-anchored activity matrix. */
export interface RegionBandCell {
  region: string
  band: string
  stations: number
  hearMe: number
  iHear: number
}

/** One ionosonde's live measured ionosphere (prop.kc2g.com). MUF/foF2 null when the
 *  station didn't report; ageSecs = how stale the reading is. */
export interface MufStation {
  lat: number
  lon: number
  mufMhz: number | null
  fof2Mhz: number | null
  ageSecs: number
  confidence: number | null
}

/** NOAA SWPC R/S/G scales (0..5): radio blackout / solar radiation / geomagnetic, now,
 *  plus tomorrow's forecast G. */
export interface NoaaScalesView {
  r: number
  s: number
  g: number
  gTomorrow: number
  /** Stamped only on a real fetch — null/absent = never loaded (an all-zero
   * default must not render as a genuinely quiet sun). */
  asOf?: number | null
}

/** One SWPC space-weather alert/watch/warning. `issued` = Unix seconds. */
export interface AlertView {
  productId: string
  issued: number
  kind: string
  message: string
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
  /** MODELED openness from physics (MUF vs band freq + absorption/aurora/greyline),
   * independent of observed spots: "Open" | "Marginal" | "Closed". Lets the UI show
   * "open per model, no spots heard" so a quiet band never reads as dead. */
  modeled?: BandModeled
  /** One-clause reason for the modeled state ("open per model" / "below MUF"). */
  modeledReason?: string
}

/** Coarse modeled band openness (collapses the engine's 5-bucket workability). */
export type BandModeled = 'Open' | 'Marginal' | 'Closed'

/** Geography-based rarity of a Maidenhead grid square (Natural Earth-derived):
 * ultraRare = open water (rover/maritime/DXpedition-only), rare = islet/sliver,
 * uncommon = mostly water or polar wilderness. */
export type GridRarity = 'common' | 'uncommon' | 'rare' | 'ultraRare'

/** Direction of a space-weather quantity's recent change. */
export type TrendDir = 'rising' | 'steady' | 'falling'

/** One scalar's current value + recent slope. */
export interface ScalarTrend {
  now: number
  deltaPerHr: number
  dir: TrendDir
}

/** Rolling space-weather trend (so the UI can say "MUF building / Kp rising"). */
export interface WxTrend {
  sfi: ScalarTrend
  kp: ScalarTrend
  muf: ScalarTrend
  xray: ScalarTrend
  windowSecs: number
  samples: number
}

/** How urgently/positively an insight reads (drives colour + ordering). */
export type InsightLevel = 'good' | 'info' | 'caution' | 'alert'

/** What a predictive insight is about (drives the icon). */
export type InsightKind =
  | 'mufTrend'
  | 'solarFlux'
  | 'geomagnetic'
  | 'flare'
  | 'greyline'
  | 'esWatch'
  | 'bandHeadroom'
  | 'openingMomentum'
  | 'reciprocity'
  | 'solarWind'

/** One plain-language predictive insight line (dual-audience: plain + technical). */
export interface Insight {
  kind: InsightKind
  level: InsightLevel
  plain: string
  technical: string
  band?: string
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
  /** Numeric combined confidence in [0,1] (the v2 detector score). */
  confidenceScore: number
  /** Far stations confirmed two-way with the operator in the window. */
  reciprocalPairs: number
  /** Onset anomaly z-score (how far above the band's own baseline). */
  anomalyZ: number
  /** Seconds since onset (stamped by the tracker in the command layer). */
  onsetSecs: number
  /** Just opened this poll — drives the one-shot alert. */
  isNew: boolean
  note: string
}
/** One completed opening episode from the persistent openings log
 * (get_openings_log; mirrors propagation::OpeningEpisode). */
export interface OpeningEpisode {
  band: string
  /** Propagation-mode label: "Tropo" | "Sporadic-E" | "Aurora" | "F2" | "Unknown". */
  mode: string
  startedUtc: number
  endedUtc: number
  durationSecs: number
  /** False when the opening was already live at startup — duration under-counts. */
  onsetKnown: boolean
  peakZ: number
  maxKm: number
  peakStations: number
  bearingDeg: number
  octant: string
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
  /** Announced modes (NG3K) — routes map click-to-work to the right cockpit.
   * Empty/missing = unannounced (treated as digital). */
  modes?: string[]
}
/** One band's outlook on a path: best workability + window + per-UTC-hour grid. */
export interface BandOutlook {
  band: string
  workability: string
  score: number
  window: string
  /** True when the best window is a short low-band greyline (terminator) spike. */
  grayline: boolean
  /** Per-UTC-hour likelihood (24 values, hour 0..23) — the heatmap row. */
  hourly: number[]
  /** Circuit reliability: % of the 24 h the band is usable (≥ Fair) — a coverage metric. */
  reliability: number
  /** Per-mode workability right now (P.533 engine only — real SNR statistics vs
   * each mode's required SNR). Empty/absent on the heuristic → the UI hides it. */
  modeNow?: { mode: string; score: number }[]
}
/** Per-path HF prediction (the PathPredictor seam): operator↔DX, best-first. */
export interface PathPrediction {
  /** Engine that produced it: "heuristic" today; "voacap"/"p533" later. */
  engine: string
  bands: BandOutlook[]
  /** Controlling MUF (MHz) on the path now — the band ceiling. */
  mufNow: number
  /** Per-UTC-hour MUF (24 values) — the ceiling line above the heatmap. */
  mufHourly: number[]
}
/** One receiver who decoded the operator ("getting out"). */
export interface HeardMe {
  call: string
  grid: string | null
  band: string
  snr: number | null
  bearingDeg: number
  km: number
  octant: string
  ageSecs: number
}
/** "Am I getting out?" — who is hearing the operator now (observed). */
export interface GettingOut {
  count: number
  maxKm: number
  reports: HeardMe[]
}
/** One OVATION aurora-oval sample (probability 0..100 %) for the map overlay. */
export interface AuroraPoint {
  lat: number
  lon: number
  prob: number
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

/** One DXpedition's modelled contact windows from YOUR grid ("Your Window") —
 * computed by the configured prediction engine (get_dxped_windows). */
export interface DxpedWindow {
  call: string
  /** Which model produced it: "p533" → the badge shows P.533, else modelled. */
  engine: string
  /** One-line headline, e.g. "17m Good 0230–0430Z" (same format as CalendarEntry.best). */
  best: string
  /** Top bands' 24 h outlooks, best first — feeds LikelihoodHeatmap. */
  outlook: BandOutlook[]
  /** Week planner: per-day best shot, index 0 = today (empty/absent for 1-day calls). */
  days?: DxpedDayBest[]
  /** Announced on-air dates (forward-calendar entries only; null = active NOW,
   * no date gate). The wake-me alarm fires only inside these. */
  startUnix?: number | null
  endUnix?: number | null
}
/** One day of the week planner: best-band headline + 0..1 score for the strip. */
export interface DxpedDayBest {
  dayUnix: number
  best: string
  score: number
}
/** Polar-cap absorption (PCA) view — GOES protons through the D-RAP2 model
 * (get_pca). Null command result = no proton data ever fetched (offline);
 * empty `points` = quiet sky (draw nothing). */
export interface PcaView {
  /** J(≥10 MeV) pfu — the NOAA S-scale driver (S1=10, S2=100, …). */
  j10: number
  /** Day/night 30 MHz polar-cap absorption (dB). */
  a30Day: number
  a30Night: number
  /** Polar-cap cutoff (geomagnetic latitude, °) at the current Kp. */
  cutoffDeg: number
  points: { lat: number; lon: number; db30: number }[]
}

/** Satellites view (get_satellites): amateur-bird subpoints now + next-24h passes
 * over the QTH. Null = no orbital elements (or all >30 d stale) — draw nothing. */
export interface SatView {
  /** Age of the oldest element set (days) — badge stale when > 14. */
  tleAgeDays: number
  birds: {
    name: string
    lat: number
    lon: number
    altKm: number
    footprintKm: number
    /** Ground track ±now ([unix, lat, lon] per minute): past = trail, future =
     * projection; the map interpolates along it for real-time motion. */
    track: [number, number, number][]
  }[]
  /** Sorted by AOS; geometry only — no transponder/workability claim. */
  passes: SatPass[]
}
export interface SatPass {
  name: string
  aosUnix: number
  losUnix: number
  maxElDeg: number
  aosAzDeg: number
  losAzDeg: number
  /** SatNOGS operational status, stamped on Satellites-section schedule rows
   * when the weekly cache has this bird; absent on the map's SatView passes
   * and whenever the cache is empty (offline) — never guessed. */
  status?: string | null
}

/** One SatNOGS DB transmitter/transponder entry (data CC-BY-SA 4.0). */
export interface SatTransmitter {
  description: string
  /** SatNOGS's curated "still transmitting" flag — measured community truth. */
  alive: boolean
  mode: string | null
  uplinkLowHz: number | null
  downlinkLowHz: number | null
}

/** Per-bird detail (get_sat_detail): SatNOGS data + the current/next pass geometry. */
export interface SatDetail {
  name: string
  /** NORAD catalog number parsed from the TLE; null if unparseable. */
  norad: number | null
  /** SatNOGS operational status ("alive"/"dead"/"re-entered"/"future");
   * null = no data yet (offline since install) — render nothing, never guess. */
  status: string | null
  transmitters: SatTransmitter[]
  /** When the SatNOGS snapshot was fetched (unix secs); null = never. */
  dataFetchedAt: number | null
  /** The pass in progress, or the next one over the operator (24 h); null = none/no grid. */
  pass: SatPass | null
  /** (unix, az°, el°) samples across `pass` for the polar plot; empty without a pass. */
  passTrack: [number, number, number][]
}

/** Live rotor auto-track state (sat_track_status); null = not tracking. */
export interface SatTrackStatus {
  name: string
  /** armed = waiting (no rotor commands until 5 min before AOS);
   * prepositioning = slewing to the AOS azimuth; tracking = following. */
  state: 'armed' | 'prepositioning' | 'tracking'
  azDeg: number
  elDeg: number
  aosUnix: number
  losUnix: number
}

/** Real-time solar wind (DSCOVR) — the leading geomagnetic indicator (leads Kp/A). */
export interface SolarWind {
  /** Bz (GSM), nT. Negative = southward = geoeffective. */
  bzNt: number
  /** Total field magnitude Bt, nT. */
  btNt: number
  /** Bulk speed, km/s. */
  speedKms: number
  /** Proton density, p/cm³. */
  density: number
}

export interface SpaceWxView {
  sfi: number
  kp: number
  aIndex: number
  xrayClass: string
  flare: boolean
  /** Raw GOES long-band (0.1–0.8 nm) X-ray flux, W/m² — the true flare magnitude
   * behind `xrayClass`/`flare` (drives the map's D-RAP flare layer). */
  xrayLong?: number
  /** Real-time solar wind; absent when the DSCOVR feed is unavailable. */
  solarWind?: SolarWind | null
}

/** The 60 s X-ray fast lane (`get_xray_now`) — fresher than the 5-min prop
 * snapshot so flare onset shows in ~1 min. */
export interface XrayNow {
  /** GOES long-band X-ray flux, W/m². */
  flux: number
  /** When the reading was fetched (Unix seconds, UTC). */
  asOf: number
}
export interface PropagationSnapshot {
  advisory: PropAdvisory
  openings: OpeningView[]
  dxpeditions: DxpedDashboard
  spaceWx: SpaceWxView
  /** Provenance: 'live' (both feeds fresh), 'partial' (some feeds live, others
   *  unreachable), 'cached' (stale last-good), or 'offline' (no live data — an
   *  honest empty snapshot; NEVER fabricated/demo data). */
  source: 'live' | 'partial' | 'cached' | 'offline'
  /** When this data was produced (Unix seconds, UTC). */
  asOf: number
  /** Located spots for the map (own-call + region + cluster/RBN + own decodes). */
  spots?: MapSpot[]
  /** "Worldwide activity" band ranking (the same advisor over the GLOBAL firehose),
   *  shown beside the operator-reachable `advisory` so a chaser sees busy-worldwide
   *  vs workable-for-you. Absent when the firehose adds nothing beyond reachable. */
  worldwide?: PropAdvisory
  /** Rolling space-weather trend (SFI/MUF/Kp/X-ray rising/steady/falling) — drives the
   *  "MUF building" insight + trend arrows. All-steady until the buffer fills. */
  wxTrend?: WxTrend
  /** Ranked plain-language predictive insights ("MUF building → 6m soon", flare, Kp,
   *  greyline, Es watch). */
  insights?: Insight[]
  /** Best band PER reachable region — the best-band recommender (operator-anchored). */
  bestToRegion?: RegionBest[]
  /** The operator-anchored (region, band) activity matrix. */
  regionBand?: RegionBandCell[]
}

/** One located spot for the map (placed by grid, or DXCC centroid if grid-less). */
export interface MapSpot {
  call: string
  lat: number
  lon: number
  band: string
  heardMe: boolean
  ageSecs: number
  approx: boolean
  /** Exact spot frequency (MHz) when the source carried one (cluster/RBN, PSKR
   * HTTP) — what map click-to-work tunes to. Null = band-level only. */
  freqMhz?: number | null
  /** Mode named by the source ("CW", "FT8", "SSB"…) — routes click-to-work to the
   * right cockpit. Null = unknown (treated as digital). */
  mode?: string | null
  /** DXCC entity name (cty.dat) — the selected-spot card's "who/where". */
  entity?: string | null
  /** Rarity of the station's grid — only for spots placed by a real grid. */
  gridRarity?: GridRarity | null
  /** CQ zone from the same resolution. */
  cqZone?: number | null
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

/** A Parks/Summits On The Air activator currently on the air (hunter feed). */
export interface OtaSpot {
  program: string
  reference: string
  name: string
  activator: string
  freqKhz: number
  mode: string
  spotter: string | null
  comment: string | null
  grid: string | null  /** This park/summit has never been logged (hunter side) — a NEW PARK. */
  newPark?: boolean
  /** Your own signal is being received on this band right now (live PSKR). */
  bandOpen?: boolean
}

/** The operator's current activation state (POTA/SOTA). */
export interface Activation {
  /** "POTA" | "SOTA", or null when not activating. */
  program: string | null
  reference: string | null
  qsoCount: number
}

/** A zero-config auto-detected USB radio (from `detect_rigs`). */
export interface DetectedRig {
  portName: string
  vid: number
  pid: number
  product: string
  manufacturer: string
  /** Hamlib model guessed from the USB product string (null = chip known, rig not). */
  suggestedModel: number | null
  suggestedModelName: string | null
  /** Bridge-chip name (e.g. "Silicon Labs CP210x") or "USB (native)". */
  chip: string
  /** Driver guidance + official link when one is needed on this OS. */
  driverNote: string | null
  driverUrl: string | null
  driverBundled: boolean
  /** Best-guess paired sound device (the rig's USB-Audio CODEC). */
  suggestedAudio: string | null
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
  /** DXCC entity name (country), resolved from the callsign. */
  country?: string | null
  /** Tier/protocol last heard on — 'TempoFast' = Tempo, 'FT8'/'FT4' = digital ops. The Tempo
   * roster shows only Tempo (TempoFast) stations; Operate shows all. */
  tier?: Tier | null
  /** Geography-based rarity of the station's grid. */
  gridRarity?: GridRarity | null
  /** Uploads to LoTW within the recency window (see DecodeRow.lotwUser). */
  lotwUser?: boolean
}

export interface ContestEvent {
  name: string
  startUnix: number
  endUnix: number
  url: string | null
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
  /** Outbound directed message the recipient acknowledged (an RR73 ACK came back) —
   * a REAL delivery confirmation, not the "a later reply implies they heard us" guess. */
  delivered?: boolean
  /** Outbound directed message still HELD in the store-and-forward queue — never yet
   * released on the air, because the recipient hasn't been heard. Cleared on its first
   * transmission. Distinguishes "going nowhere" from "sent, awaiting ACK". */
  stored?: boolean
  /** Outbound directed message that was still HELD when the app last closed. The
   * store-and-forward queue does not survive a restart, so it will NEVER transmit —
   * the operator must re-send. Shown as "Not sent" rather than a misleading "Sent". */
  abandoned?: boolean
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
  /** RF output power 0.0–1.0: rig read-back when CAT reports it, else the last
   * commanded value; absent until either exists. */
  rfPower?: number | null
  /** Mic gain as a 0.0–1.0 fraction (rig read-back, or the commanded value). Absent when
   * the rig doesn't report it. Pairs with the ALC meter for SSB mic-gain setup. */
  micGain?: number | null
  /** Noise-reduction level 0.0–1.0 (rig read-back or commanded); absent when unsupported. */
  nrLevel?: number | null
  /** AGC time constant: "fast" | "mid" | "slow"; absent when the rig doesn't report it. */
  agc?: string | null
  /** CAT S-meter in dB relative to S9 (S9 = 0, S1 ≈ -48, S9+20 = +20). Absent when
   * the rig doesn't report STRENGTH over CAT (RX-only; not updated during TX). */
  smeterDb?: number | null
  /** Transmit meters from the rig's CAT poll — the mirror of smeterDb: present ONLY while
   * transmitting, absent while receiving (or when the rig doesn't report that meter).
   * txSwr = SWR ratio (1.0–6.0), txAlc = ALC 0–1, txPoW = output watts, txCompDb = COMP dB. */
  txSwr?: number | null
  txAlc?: number | null
  txPoW?: number | null
  txCompDb?: number | null
  /** The rig's actual mode read back over CAT (e.g. "USB"/"LSB"/"FM"); display-only,
   * used to flag when the rig's mode knob disagrees with the commanded mode. */
  rigMode?: string | null
  /** A CAT read succeeded — dial/mode are the rig's own values, not the persisted seed. */
  rigConfirmed?: boolean
  /** Transient Phone mode override ("USB"/"LSB"/"FM"), or null/absent = AUTO (band-derived). */
  sidebandOverride?: string | null
  /** The operator's phone (SSB) sub-band on the current band as [lo, hi) MHz, per license class
   * — the band-strip shades it. Both absent for no-phone-privilege / Open / off-plan bands. */
  phoneSegLo?: number | null
  phoneSegHi?: number | null
  /** Rig DSP-function states over CAT; null/absent = the rig doesn't report it (hide the toggle).
   * `notch` is the auto-notch (ANF). Same null=unsupported idiom as smeterDb. */
  nb?: boolean | null
  nr?: boolean | null
  notch?: boolean | null
  comp?: boolean | null
  vox?: boolean | null
  /** Rig RX passband / filter width in Hz over CAT; null/absent = unknown or the rig's default. */
  filterWidthHz?: number | null
  /** RIT (receive incremental tuning) offset in Hz — last commanded (0 = off). */
  ritHz?: number
  /** XIT (transmit incremental tuning) offset in Hz — last commanded (0 = off). */
  xitHz?: number
  /** Active VFO ("A" / "B") — last commanded. */
  activeVfo?: string
  rxLevel: number
  /** Whether transmit is enabled (Monitor on). Off = muted/listening only. */
  txEnabled: boolean
  /** Whether the operator's license class permits TX at the current dial+mode. False = TX
   * hard-blocked (outside privileges); the cockpit shows a lock indicator. */
  txAllowed: boolean
  /** Whether a tune carrier is currently keyed. */
  tuning: boolean
  /** True if the TX watchdog has auto-halted transmit (needs a re-enable). */
  txWatchdog: boolean
  /** Whether a QSO recording (audio bridge) is streaming live RX to disk. Persists across
   * nav — drives the Phone cockpit's REC badge. */
  qsoRecording: boolean
  /** Rig/CAT health: null/undefined = N/A (VOX); true = connected; false = failing. */
  catOk?: boolean | null
  /** Human-readable rig/CAT status (read frequency, or a specific error). */
  catDetail?: string
  /** The CW keyer backend the engine is actually using: 'cat' (rig in CW) or
   * 'soundcard' (rig in USB/LSB). Lets the CW cockpit toggle show the REAL state. */
  cwKeyer?: string
  /** The keyer speed (WPM) the engine is actually using — the cockpit slider's
   * source of truth across navigation. */
  cwWpm?: number
  /** Rig split TX dial (MHz) when a pile-up spot configured split; null/absent =
   * simplex. Drives the SPLIT badge. */
  splitTxMhz?: number | null
  /** Set when the sound card failed to open (explains a blank waterfall). */
  audioError?: string | null
  /** Set when two radios are on the same serial COM port (explains a red pill). */
  radioConfigWarning?: string | null
  /** Transmit on even/"1st" slots (true) or odd/"2nd" (false). */
  txEven: boolean
  /** Smart auto-cycle on: answering a heard station auto-picks the opposite cycle
   * (FT8-style). False = the operator fixed the cycle manually. */
  txCycleAuto?: boolean
  /** Active T/R period (s) — TempoFast 4s, FT8 15s, FT4 7.5s — so the UI labels the cycle
   * with the real period. */
  trPeriodSecs?: number
  /** Presence heartbeat on — a periodic beacon so listening stations are deliverable
   * (drives the Tempo Heartbeat toggle). */
  beacon?: boolean
  /** Receive audio offset (Hz) — the green waterfall marker. */
  rxOffsetHz: number
  /** Transmit audio offset (Hz) — the red waterfall marker. */
  txOffsetHz: number
  /** Keep the TX offset fixed when RX changes ("Hold Tx Freq"). */
  holdTxFreq: boolean
  /** TX audio drive level (0.0–1.0) — the "Pwr" slider. */
  txLevel: number
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

/** A single-signal CW decode of the recent RX audio (live readout). */
export interface CwDecodeResult {
  text: string
  wpm: number
  /** TX echo: recent expanded CW transmissions (oldest→newest) — what actually went out. */
  sent: string[]
  /** A CW-keyer failure to surface (e.g. the rig rejected CAT keying), else null. */
  keyerError: string | null
  /** CW copilot: ranked worked-station callsign candidates from the decode (click to confirm). */
  candidates: { call: string; best: boolean }[]
  /** RST they sent us, read from the decode (e.g. "599"), else null. */
  rst: string | null
  /** The other station's name, read from the decode (e.g. "BOB"), else null. */
  name: string | null
  /** Guided QSO-state tag: "listening" | "cq" | "answered" | "report" | "73". */
  state: string
  /** Plain-English state, e.g. "W1ABC is calling CQ". */
  headline: string
  /** Guided instruction, e.g. "Press Answer (F2) to call them". */
  prompt: string
  /** Recommended action id to highlight: "F2" | "F3" | "log", or null. */
  recommended: string | null
  /** The operator-confirmed worked callsign (active peer), if any. */
  workedCall: string | null
}

/** One signal found by the wideband CW skimmer (audio pitch + text + WPM). */
export interface SkimHit {
  pitchHz: number
  text: string
  wpm: number
}

/** Live RTTY RX decoder state (poll get_rtty_state while the RTTY cockpit is
 * visible). RX decode only — arming never touches any TX path. */
export interface RttyState {
  armed: boolean
  /** AFC offset from the nominal mark/space pair (Hz). */
  afcHz: number
  /** AFC has acquired-then-frozen on a signal. */
  afcLocked: boolean
  /** Current mark/space audio tones (Hz) the demod is netted on — the waterfall
   * mark/space cursor positions. Un-netted = the nominal 2125/2295 pair. */
  markHz: number
  spaceHz: number
  /** The decoded-text ring tail (caps at ~4000 chars; oldest drop off). */
  text: string
  /** Per-character confidence 0–100, parallel to `text`'s chars — render low
   * values faint (the ATC soft metric). */
  charConf: number[]
  /** Configured baud rate (true 45.45 by default). */
  baud: number
  /** Configured mark/space shift (Hz). */
  shiftHz: number
  /** Keying backend: 'afsk' (soundcard tones, rig in LSB) | 'fsk' (serial keyline,
   * rig in RTTY mode). */
  backend: string
  /** An RTTY over is on the air or queued behind one (the TX indicator). */
  sending: boolean
  /** A keyer failure to surface (FSK port wouldn't open / rig refused PTT), else null. */
  keyerError: string | null
  /** The RTTY auto-sequencer is active (the operator's Auto toggle is on). */
  auto: boolean
  /** Live sequencer state: 'idle' | 'calling_cq' | 'answering' | 'exchange_sent' |
   * 'confirmed' | 'done'. */
  seqState: string
  /** The station currently being worked, once one is locked, else null. */
  peer: string | null
  /** The peer's copied exchange in schema order, as [key, value] pairs. */
  peerExchange: [string, string][]
  /** A CQ surfaced from the transcript for the operator to click-to-answer (only
   * while Auto is on), else null. Surfacing only — clicking it is the human gate. */
  heardCq: string | null
}

/** One saved SSTV image in the local gallery (a BMP in the sstv-gallery folder
 * of the Nexus local data dir, beside its gallery.json metadata). */
export interface SstvGalleryEntry {
  /** Absolute path of the saved image file. */
  path: string
  /** Mode label, e.g. "Scottie 1". */
  mode: string
  /** ISO-8601 UTC completion time, e.g. "2026-07-17T15:30:00Z". */
  finishedUtc: string
  /** Dial frequency (MHz) when the image finished. */
  freqMhz: number
  /** Decoded scan lines (= image height). */
  lines: number
  /** Decoded FSK callsign ID trailing the image (best-effort), if recovered. */
  fskId?: string | null
}

/** Live SSTV RX state: armed flag, in-flight decode progress + preview, and
 * the saved-image gallery (oldest first). RX decode only. */
export interface SstvState {
  armed: boolean
  /** Mode label while an image is in flight, else null. */
  mode: string | null
  linesDone: number
  linesTotal: number
  /** Base64 of previewWidth×previewHeight×3 raw RGB bytes (the in-progress
   * thumbnail, ≤160 px wide), else null. */
  previewRgbBase64: string | null
  previewWidth: number
  previewHeight: number
  gallery: SstvGalleryEntry[]
  /** An image is queued or streaming to the rig (the cockpit's TX indicator). */
  sending: boolean
  /** Mode label of the image being (or last) transmitted, else null. */
  txMode: string | null
  /** TX progress 0..1 (elapsed / total key-down), 0 when idle. */
  txProgress: number
  /** Seconds of key-down elapsed / total for the in-flight image. */
  txElapsedSecs: number
  txTotalSecs: number
}

/** Result of "Auto-test ports": the working (port, baud, Hamlib model) the prober
 * auto-selected, or found=false with a detail message. */
export interface CatProbeResult {
  found: boolean
  portName: string
  baud: number
  model: number
  modelName: string
  freqMhz: number
  detail: string
  /** The model was a GUESS (a seeded common-rig probe) — apply port/baud but keep picking Rig Model. */
  modelSeeded?: boolean
}

export interface Spectrum {
  row: number[]
  /** The window the row spans (Hz) — data-driven, never hardcode 200/2900. For a native RF
   * panadapter this is the absolute RF span. */
  loHz?: number
  hiHz?: number
  /** Feed source: "audio" (soundcard FFT), "flex" (SmartSDR VITA), or "civ" (Icom CI-V scope). */
  source?: string
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
  /** Sender's DXCC entity name (country), resolved from the callsign. */
  country?: string | null
  /** Sender resolves to a DXCC entity never worked before (a "new one"). */
  newDxcc?: boolean
  /** Decode carries a Maidenhead grid never worked before. */
  newGrid?: boolean
  /** The grid the decode carried (CQ/grid messages) — for alert copy + rarity. */
  grid?: string | null
  /** Geography-based rarity of that grid (rare ones alert loudly). */
  gridRarity?: GridRarity | null
  /** The sender uploads to LoTW within your recency window (Settings) — the
   * "this contact will confirm" mark. False until the ARRL list is fetched. */
  lotwUser?: boolean
  /** True if this row is OUR OWN transmitted message (yellow, one per cycle). */
  mine?: boolean
  /** For `mine` rows: Unix-second the message was transmitted — the stable
   * per-cycle key + timestamp (so own-TX rows don't drift/duplicate). */
  txAt?: number | null
  tier: Tier
  /** WSJT-X 'a' marker — AP-assisted decode. */
  ap?: boolean
  /** WSJT-X '?' marker — low-confidence decode. */
  lowConf?: boolean
  /** IR-HARQ redundancy versions combined to recover this decode: 0 = decoded
   * from the initial transmission alone; 1/2 = recovered by joint-combining that
   * many retransmissions; -1 = not applicable. Used to badge HARQ rescues. */
  rv: number
}

/** A logged contact in the general ADIF logbook (separate from Field Day). */
export interface LoggedQso {
  call: string
  grid: string | null
  /** DXCC entity name (country), resolved from the callsign — the key DXer field. */
  country?: string | null
  /** US state (ADIF STATE, 2-letter) for WAS, when known. */
  state?: string | null
  band: string
  freqMhz: number
  mode: string
  /** Signal report as a string: CW "599" / phone "59" / digital "-12" dB. */
  rstSent: string | null
  rstRcvd: string | null
  /** Operator name (ADIF NAME) — callbook autofill / ragchew logging. */
  name?: string | null
  /** QSO location / city (ADIF QTH). */
  qth?: string | null
  /** Short sharable remark (ADIF COMMENT). */
  comment?: string | null
  /** Free-form multi-line operator notes (ADIF NOTES). */
  notes?: string | null
  /** Transmit power in watts (ADIF TX_PWR). */
  txPower?: number | null
  /** Contact time, seconds since the Unix epoch (UTC). */
  whenUnix: number
  /** Confirmed via ANY channel (LoTW / eQSL / paper QSL). */
  confirmed: boolean
  /** Award-eligible confirmation (LoTW or paper only — eQSL excluded). */
  awardConfirmed: boolean
  /** WHICH channel(s) confirmed — per-source truth behind the booleans
   * (all-false on legacy records whose sync predates the split). */
  qslRcvd?: { card: boolean; lotw: boolean; eqsl: boolean; qrz?: boolean }
  /** Operator-declared OUTBOUND QSL-request state (did I send a card, how, when).
   * `via` is the ADIF QSL_SENT_VIA letter: "B"(ureau) / "D"(irect) / "E"(lectronic).
   * A request is NOT a confirmation. */
  qslSent?: { sent: boolean; via: string | null; dateUnix: number | null }
  /** Awards credit granted by ARRL (normalized ADIF codes, e.g. "DXCC"). */
  creditGranted?: string[]
  /** Awards credit applied/submitted but not yet granted. */
  creditSubmitted?: string[]
  /** Per-source outbound upload state (drives the "Upload to LoTW" count). */
  upload?: UploadState
  /** POTA/SOTA references — my activation (my*) and the station I hunted (their*). */
  ota?: Ota
}

/** On-the-air (POTA/SOTA) references for a QSO. POTA maps to ADIF SIG/SIG_INFO, SOTA to SOTA_REF. */
export interface Ota {
  myProgram?: string | null
  myRef?: string | null
  theirProgram?: string | null
  theirRef?: string | null
}

/** Per-source upload status (mirror of the Rust UploadStatusDto). */
export interface UploadStatus {
  /** "pending" | "accepted" | "duplicate" | "rejected" | "authfail". */
  outcome: string
  whenUnix: number
  detail?: string | null
}
export interface UploadState {
  lotw?: UploadStatus
  eqsl?: UploadStatus
  qrz?: UploadStatus
  clublog?: UploadStatus
}
/** Result of a LoTW upload (TQSL sign+upload). */
export interface UploadReport {
  dispatched: number
  /** "pending" | "duplicate" | "rejected" | "authfail" | "retry" | "none". */
  outcome: string
  detail?: string | null
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
  /** Newly confirmed by any channel (incl. eQSL) — the eQSL sync's headline count
   *  (newlyConfirmed is award-grade and always 0 for eQSL). */
  newlyConfirmedAny: number
  newlyCredited: number
  newlySubmitted: number
  /** Uploads the own-echo pull promoted Pending→Accepted (your side now on file).
   *  0 for a paste-reconcile (only the online sync runs the own-echo pull). */
  promoted: number
  /** New QSOs pulled down and added to the log (QRZ two-way sync only). 0 otherwise. */
  added?: number
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

// --- Journey: the in-app, beginner-first achievement layer (get_journey) ---

export type JourneyTier = 'bronze' | 'silver' | 'gold' | 'platinum' | 'legendary'

/** An auto-detected "first" — the hobby's biggest unfilled recognition gap. */
export interface JourneyFirst {
  id: string
  title: string
  /** Plain "what it means for the operator". */
  meaning: string
  /** A sentence of ham heritage/context. */
  heritage: string
  unlocked: boolean
  /** When it happened (Unix s), once unlocked. */
  whenUnix: number | null
  detail: string | null
}

/** A named rung on a sub-award ladder. */
export interface JourneyRung {
  label: string
  target: number
  tier: JourneyTier
}

/** A tiered ladder climbing toward a big official award. */
export interface JourneyLadder {
  id: string
  title: string
  meaning: string
  heritage: string
  worked: number
  confirmed: number
  rungs: JourneyRung[]
  /** Nearest unmet rung by worked count (the "N to go" target). */
  nextRung: JourneyRung | null
  max: number
}

export interface JourneyCell {
  key: string
  label: string
  worked: boolean
  confirmed: boolean
}

export interface JourneyCollection {
  id: string
  title: string
  meaning: string
  cells: JourneyCell[]
  worked: number
  total: number
}

export interface JourneyFeat {
  id: string
  title: string
  meaning: string
  heritage: string
  tier: JourneyTier
  unlocked: boolean
  current: number
  target: number
  unit: string
  detail: string | null
  /** True when it can't be evaluated yet (e.g. miles-per-watt with no power set). */
  gated: boolean
  gateHint: string | null
}

export interface JourneyPersonalBest {
  id: string
  title: string
  value: string
  detail: string | null
}

export interface JourneyStreak {
  enabled: boolean
  weeks: number
  activeThisWeek: boolean
}

export interface JourneyNextMilestone {
  ladderId: string
  title: string
  current: number
  target: number
  remaining: number
}

/** The full Journey snapshot (the in-app achievement layer). */
export interface JourneySummary {
  level: number
  xp: number
  xpIntoLevel: number
  xpForLevel: number
  totalQsos: number
  nextMilestone: JourneyNextMilestone | null
  firsts: JourneyFirst[]
  ladders: JourneyLadder[]
  collections: JourneyCollection[]
  feats: JourneyFeat[]
  bests: JourneyPersonalBest[]
  streak: JourneyStreak
  /** Annual personal DX marathon (entities + zones worked THIS calendar year). */
  marathon?: JourneyMarathon
}

/** The annual marathon scoreboard — resets every Jan 1, best year remembered. */
export interface JourneyMarathon {
  year: number
  entities: number
  zones: number
  score: number
  bestYear: number | null
  bestScore: number
}

/** DXCC-first award progress, computed from the logbook (cty.dat-resolved). */
/** Why a heard station is worth working (need-aware spotting). */
export type NeedTag =
  | 'NewEntity'
  | 'NewZone'
  | 'NewBand'
  | 'NewMode'
  | 'NewGrid'
  | 'NewState'
  | 'Confirm'
  | 'Dxped'
  | 'Pota'
  | 'Sota'
  | 'Wanted'

/** One phone voice-keyer slot: an F-key-numbered label bound to a recorded WAV.
 * `file` is empty until the operator records or imports a message. */
export interface VoiceMessage {
  slot: number
  label: string
  file: string
}

/** A scored need opportunity for a station heard right now. */
export interface NeedAlert {
  call: string
  entity: string
  band: string
  zone: number
  tags: NeedTag[]
  priority: number
  headline: string
  /** Operating-mode class — 'CW' | 'Phone' | 'Digital' | 'RTTY'. Routes a click-to-work to
   * the matching cockpit and drives the row's mode badge. 'RTTY' is a Digital submode label
   * (RBN skimmer spots only) that routes to the RTTY cockpit; it filters as Digital. */
  mode: string
  /** Exact spot frequency in MHz when known (cluster/RBN), else null (band-level
   * reception needs). Lets click-to-work QSY to the spot, not just the band default. */
  freqMhz: number | null
  /** Unix seconds of the most recent admitting evidence — drives "N min ago". */
  admittedAt?: number | null
  /** The board shows its work: "heard by K9LC (EN52, 26 km) + N9CO (62 km)". */
  evidence?: string | null
  /** Geography-based rarity of the heard grid (when the source carried one) —
   * drives the gem + a NewGrid priority boost. */
  gridRarity?: GridRarity | null
}

/** One raw cluster/RBN spot for the Spots panel (the SpotCollector-style firehose).
 *  NOT needs-gated — every recent spot; the panel filters client-side. */
export interface SpotRow {
  call: string
  /** DXCC entity, '' if unresolved. */
  entity: string
  /** CQ zone, 0 if unknown. */
  zone: number
  /** Band label ('20m'), '' if off the band plan. */
  band: string
  freqMhz: number
  /** 'CW' | 'Phone' | 'Digital'. */
  mode: string
  spotter: string
  /** Other spotters of the same DX (multi-endpoint evidence). */
  corroborators: string[]
  /** Seconds since received; -1 if unknown. */
  ageSecs: number
  comment: string
  /** Operator may transmit at this freq+mode (license privileges; Open class ⇒ true). */
  licensed: boolean
}

/** A QRZ.com callsign-lookup result. grid/state are subscriber-only and routinely
 *  null for free QRZ accounts. */
export interface QrzLookup {
  call: string
  name: string | null
  /** QRZ nickname — shown in place of the full name when set (null for HamQTH). */
  nickname: string | null
  qth: string | null
  grid: string | null
  state: string | null
  country: string | null
  dxcc: number | null
  cqZone: number | null
  ituZone: number | null
  /** Profile photo URL (QRZ image / HamQTH picture) — routinely null. */
  image: string | null
}

/** Result of a QRZ Logbook push (one-QSO upload). `result` is the outcome tag;
 *  `duplicate` is the benign "already in your QRZ logbook". */
export interface QrzPushResult {
  result: 'ok' | 'replace' | 'duplicate' | 'authFail' | 'fail'
  logid: string | null
  reason: string | null
}

/** Result of a ClubLog realtime push. `duplicate` is the benign "already on
 *  ClubLog"; `authFail` means a 403 (auto-upload is then suppressed until creds change). */
export interface ClubLogPushResult {
  result: 'ok' | 'modified' | 'duplicate' | 'rejected' | 'authFail' | 'serverError' | 'unknown'
  message: string | null
}

/** Result of an HRDLog.net upload. `duplicate` is the benign "already on HRDLog";
 *  `authFail` means a bad callsign/upload code. HRDLog.net is a live-logging/awards
 *  site — it is NOT an ARRL confirmation source (never DXCC/WAS credit). */
export interface HrdLogPushResult {
  result: 'ok' | 'duplicate' | 'authFail' | 'rejected' | 'unknown'
  message: string | null
}

/** Liveness of one background live feed, for the Now-Bar connector pills. */
export interface FeedStatus {
  /** The feed's daemon is running. Started once a real callsign (and, for the
   *  cluster, its toggle) is set, then runs until app exit — so it can stay true
   *  after the cluster toggle is later turned off. When false the UI hides the pill. */
  enabled: boolean
  /** Seconds since the last parsed spot/report; null if none yet this session. */
  lastEventSecs: number | null
  /** Only meaningful when `enabled`. 'connected' = session up but quiet (normal —
   * NOT broken); 'connecting' = no session yet; 'reconnecting' = had events, session
   * currently down. ('waiting' is the legacy pre-connected-flag label.) */
  state: 'off' | 'connecting' | 'connected' | 'live' | 'idle' | 'reconnecting' | 'waiting'
}

/** One connectivity event for the Settings ▸ Connections log. */
export interface ConnEvent {
  tsUnix: number
  connector: string
  level: 'ok' | 'info' | 'error' | string
  message: string
}

/** Whether a secret is stored for a connector (never the secret itself). */
export interface CredStatus {
  connector: string
  stored: boolean
  identity: string
}

/** Liveness of the background live feeds (DX cluster/RBN + PSK Reporter MQTT). */
export interface FeedHealth {
  cluster: FeedStatus
  pskr: FeedStatus
  /** The human DX-cluster node alone — the SSB/phone source, separate from `cluster`
   * (which the RBN CW/digital firehose keeps green on its own). `enabled: false` when
   * no human node is configured (RBN-only operator). */
  phoneCluster: FeedStatus
  /** The configured human DX-cluster host (e.g. "ve7cc.net:23") for the phone-source
   * label; null when no human node is configured. */
  phoneClusterHost: string | null
  /** PHONE-classed spots received from human nodes this session — for the Needed board's
   * "N SSB spots" diagnostic (0 = SSB not arriving; >0 with no phone rows = arriving but
   * not a need). */
  phoneSpotsSeen: number
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

/** A structured fix-action for a confirmation diagnostic (only the fields
 * relevant to `kind` are present). */
export interface DiagAction {
  kind: string
  source?: string
  detail?: string
  field?: string
  found?: string
  expected?: string
  logged?: string
  suggested?: string
  call?: string
  otherIndex?: number
  untilUnix?: number
}
export interface DiagReason {
  code: string
  confidence: string
  explanation: string
  action: DiagAction
}
export interface QsoDiagnosis {
  index: number
  award: string
  status: string
  reasons: DiagReason[]
}
export interface DiagActionBucket {
  kind: string
  count: number
  qsoIndices: number[]
}
/** One entity a single award-grade fix away from a new slot / new entity. */
export interface OneAway {
  entity: string
  bands: string[]
  newEntity: boolean
}
/** "Why isn't this QSO confirmed" diagnostics report (Phase 1a). */
export interface DiagnosticsReport {
  diagnoses: QsoDiagnosis[]
  buckets: DiagActionBucket[]
  oneAway: OneAway[]
  waitingOnPartner: number
  pendingLag: number
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

/** QSOs worked on one WAC continent, with the distinct-entity span. */
export interface ContinentTally {
  /** WAC code: NA / SA / EU / AS / OC / AF. */
  continent: string
  qsos: number
  /** Distinct DXCC entities worked on this continent. */
  entities: number
}

/** QSOs worked in one CQ zone (1–40). */
export interface ZoneTally {
  zone: number
  qsos: number
}

/** The geographic slice of the logbook (from `get_log_stats`, cty.dat-resolved per callsign).
 * Descriptive, not award credit — WAE/CQ-only entities count too. Named `GeoLogStats` to stay
 * distinct from `features/logStats.ts`'s `LogStats` (the frontend-computed dashboard shape). */
export interface GeoLogStats {
  /** All QSOs scanned. */
  total: number
  /** QSOs whose callsign resolved to a DXCC entity (the base for every breakdown). */
  resolved: number
  /** Resolved QSOs with an entity different from the operator's own (DX). */
  dx: number
  /** Resolved QSOs with the operator's own entity (domestic). */
  domestic: number
  /** By WAC continent, canonical NA→AF order (empties omitted). */
  byContinent: ContinentTally[]
  /** By CQ zone, ascending; only zones with a QSO. */
  byZone: ZoneTally[]
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
  /** On-air text of the message queued for the next TX slot ("Now sending"). */
  txNow?: string | null
  /** True when the sequencer has retransmitted to its limit without the partner
   * advancing — withholding further TX until Resend or a new decode. */
  stalled?: boolean
  /** Times the current message has been sent this step ("called them N times"). */
  txCount?: number
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
  band: string  /** Scoring class: 'DIG' | 'CW' | 'PH'. */
  mode?: string
  whenUnix?: number
}

/** Field Day operating + scoring status. */
export interface FieldDayStatus {
  myClass: string
  mySection: string
  running: boolean
  state: string
  /** The station currently being worked (quiets decode popups about them). */
  dxcall?: string | null
  qsoCount: number
  sections: number
  /** The distinct sections worked (identities behind `sections`), for the board. */
  workedSections?: string[]
  points: number
  log: FieldDayQso[]  /** Which event: 'arrlfd' | 'wfd'. */
  event?: string
  /** QSO points × the power multiplier. */
  poweredPoints?: number
  /** Claimed bonus points. */
  bonusPoints?: number
  /** poweredPoints + bonusPoints. */
  totalScore?: number
}

/** Result of the SourceForge update check (Phase 1: notify + open the download page). */
export interface UpdateInfo {
  /** The running build's version. */
  current: string
  /** Latest version parsed from best_release.json, or null if it couldn't be read. */
  latest: string | null
  /** True only when `latest` is strictly newer than `current`. */
  updateAvailable: boolean
  /** The SourceForge download page to open. */
  downloadUrl: string
}

/** Persistent operator + radio settings. */
export interface Settings {
  mycall: string
  mygrid: string
  /** Operator first name — the CW {NAME} macro + logging. */
  opName: string
  /** Operator US state/province — the CW {MYSTATE} macro (ragchew QTH exchange). */
  opState: string
  /** LEGACY single DX-cluster node (host:port) — kept for back-compat; `clusterHosts` is
   * the live source of truth (the backend seeds the list from this on upgrade). */
  clusterHost: string
  /** DX-cluster nodes (host:port) — the SSB/phone aggregator. We connect to ALL of them
   * and union their human spots; RBN CW/digital connect automatically. */
  clusterHosts: string[]
  /** Companion-mode UDP listen address (WSJT-X/JTDX). */
  companionAddr: string
  /** CW keyer backend: 'cat' | 'serial' | 'winkeyer' | 'soundcard'. Persisted (Rust `cwKeyer`);
   * the CW cockpit also edits it live. Optional here as older saved settings may omit it. */
  cwKeyer?: string
  /** CW sidetone/tone pitch (Hz) — the soundcard keyer tone + the CW scope marker. */
  cwPitchHz: number
  /** Serial port for the K1EL WinKeyer (when the CW keyer backend is WinKeyer). */
  winkeyerPort: string
  /** Serial port for the DTR/RTS CW keyline (when cwKeyer === 'serial') — a SEPARATE port
   * from CAT, into the rig's KEY jack. */
  cwKeyPort?: string
  /** Which line the serial keyline toggles: 'dtr' (default, CW convention) or 'rts'. */
  cwKeyLine?: string
  /** RTTY keying backend: 'afsk' (default — soundcard tones, rig in LSB) or 'fsk'
   * (true FSK serial keyline, rig in RTTY mode — unlocks its narrow filters). */
  rttyBackend?: string
  /** Which control line carries the FSK data bits: 'dtr' (default) or 'rts'.
   * PTT rides its OWN path (CAT or the PTT serial line) — never this line. */
  rttyFskLine?: string
  /** Serial port for the FSK keying line (e.g. the FTDX10's USB Enhanced COM).
   * Empty = the CAT serial port. */
  rttyFskPort?: string
  /** RTTY baud rate — true 45.45 by default (never 45); 75 is the other common rate. */
  rttyBaud?: number
  /** RTTY mark/space shift in Hz (170 = the HF standard). TX + RX demod both. */
  rttyShiftHz?: number
  /** Reverse the mark/space sense (TX tones + RX demod) — for running the
   * opposite sideband (e.g. AFSK in USB/DATA-U). */
  rttyReverse?: boolean
  band: string
  dialMhz: number
  sideband: string
  /** Phone sub-mode: 'ssb' (sideband by band) or 'fm' (FM voice + repeater shift/CTCSS). */
  phoneMode: string
  /** FM repeater shift: 'simplex' | 'plus' | 'minus'. */
  rptrShift: string
  /** FM CTCSS (PL) tone in Hz; 0 = off. */
  ctcssToneHz: number
  /** FM repeater offset override in Hz (0 = band convention). Set by the
   * Program section's tune-now for odd-split machines. */
  rptrOffsetOverrideHz?: number
  fdClass: string
  fdSection: string
  /** The current operator at the key (call/initials) — FD rotates ops, so this
   * differs from mycall. Pushed to N3FJP as the QSO operator; empty = use mycall. */
  fdOperator?: string
  /** Field Day master switch (persisted). Default false; ONLY the operator's toggle sets it
   * true (never date/default). Drives all Field Day visibility (nav item, FD-detail tab) and
   * the backend operating mode. */
  fdActive: boolean
  /** Amateur license class: 'technician' | 'general' | 'extra' | 'open' (no TX limits). */
  licenseClass: string
  /** Active operating mode ('digital' | 'phone' | 'cw') — set live via the section nav, but
   * declared here so the settings round-trip ({...form}) preserves it on Save. */
  operatingMode?: string
  /** Phone voice-keyer slots — declared so a settings Save round-trips them (don't wipe). */
  voiceMessages?: VoiceMessage[]
  // --- rig control ---
  /** PTT keying method: CAT (rigctld) / serial RTS / serial DTR / VOX. */
  pttMethod: string
  /** Hamlib rig model number (0 = none / not selected). */
  rigModel: number
  /** Human-readable rig model name, paired with rigModel. */
  rigModelName: string
  /** Serial/COM port for rig control (e.g. "COM3" / "/dev/ttyUSB0"). */
  serialPort: string
  /** Dedicated serial/COM port for RTS/DTR PTT when it differs from the CAT port
   * (SO2R controllers key on their own COM port). Empty = key on `serialPort`. */
  pttSerialPort: string
  /** Serial baud rate. */
  baud: number
  /** Rig connection: "serial" (default) or "network" (rigctld → rigAddr over TCP, e.g. a
   * FlexRadio via SmartSDR). */
  rigConn: string
  /** Network rig address host:port when rigConn === "network" (e.g. "192.168.1.50:4992"). */
  rigAddr: string
  /** Let Nexus set the rig's mode (forces the DATA submode). Off by default —
   * Nexus obeys whatever mode the rig is already in (max compatibility). */
  /** TCP port that rigctld listens on / Tempo launches it with. */
  rigctldPort: number
  /** Native Icom CI-V: Nexus owns the CI-V serial port itself (real scope waveform +
   * instant dial tracking) instead of launching rigctld. Per-radio; default off. */
  icomNativeCat: boolean
  /** Antenna rotator: rotctld daemon `host:port` (empty = no rotator). */
  /** Integrated rotator: Hamlib rotator model # (0 = none) + serial port +
   * baud — Nexus launches the bundled rotctld itself, like the rig. */
  rotatorModel?: number
  rotatorPort?: string
  rotatorBaud?: number
  /** ADVANCED: external rotctld host:port override (wins over the integrated
   * spawn). Empty + model 0 = no rotator. */
  rotatorHost: string
  /** Run the rigctld-compatible CAT broker so other apps share the radio. */
  catBroker: boolean
  /** TCP port the CAT broker listens on (Hamlib NET rigctl default 4532). */
  catBrokerPort: number
  /** A FlexRadio's IP for the SmartSDR API (port 4992), for the native panadapter. Empty = off. */
  flexRadioIp: string
  /** Opt-in to the Flex native SmartSDR panadapter (unverified on hardware; off by default). */
  flexNativePan: boolean
  /** Let a broker client (WSJT-X/N1MM) key PTT when Nexus is idle. OFF by
   * default — Nexus owns TX unless the operator opts in. */
  catBrokerPtt?: boolean
  // --- audio ---
  /** Input (RX) audio device name. "" = system default. */
  audioIn: string
  /** Output (TX) audio device name. "" = system default. */
  audioOut: string
  /** Microphone device for RECORDING voice-keyer messages. "" (default) = record from
   * audioIn (the shared input) — but on a digital setup that's the rig's RX codec, so a
   * voice message would capture the band, not the operator. Set to the real mic and each
   * recording opens a separate input on it; a device that fails to open falls back to
   * the shared input. The decode path is never affected. */
  voiceMicDevice?: string
  /** Transmit drive level, 0–1 (default 0.9). */
  txLevel: number
  /** RX capture gain, ≥1.0 multiplier on received audio before decode (default 1.0). */
  rxGain: number
  /** Headphone monitor (DARK, off by default): live pass-through of the RX audio the
   * decoder hears to a chosen output device. Refuses to open on the rig's TX device. */
  monitorEnabled?: boolean
  /** Headphone-monitor output device name. "" = system default output. */
  monitorDevice?: string
  /** Headphone-monitor playback level, 0–1 (default 0.5). */
  monitorLevel?: number
  /** Station transmit power in WATTS (RF out) — unlocks the Journey miles-per-watt
   * + QRP feats. `null` until set (those feats stay gated). */
  stationPowerW?: number | null
  /** Opt-in: track a gentle weekly "on the air" streak on the Journey board. */
  journeyStreakEnabled?: boolean
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
  /** Append every decode to a WSJT-X-format ALL.TXT decode log (loggers/GridTracker tail it). */
  writeAllTxt: boolean
  /** Auto-save a WAV of the recent RX audio when a QSO is logged (per-contact recording). */
  saveQsoWav: boolean
  /** Log each QSO to Ham Radio Deluxe Logbook over its QSO-Forwarding UDP port. */
  hrdLogging: boolean
  /** HRD Logbook QSO-Forwarding address (UDP); HRD default 127.0.0.1:2333. */
  hrdUdpAddr: string
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
  /** Prompt to confirm/edit a completed QSO before logging (WSJT-X "Prompt me
   * to log QSO"). No effect unless autoLog. */
  promptToLog?: boolean
  /** Roger the final report with a bare RRR (partner still owes a 73) instead of
   * the combined RR73. Off by default (RR73 — modern FT8 practice). */
  preferRrr?: boolean
  /** Stop a CQ run after N unanswered calls; null/undefined = stock WSJT-X
   * (CQ repeats until you stop it — the Tx watchdog is the backstop). */
  cqMaxCalls?: number | null
  /** Auto-CQ run: abandon a caller who answered then went silent, after N unanswered
   * overs, and resume CQ. null/undefined = default (3); 0 = never abandon (wait for you). */
  cqStallOvers?: number | null
  /** WSJT-X Settings ▸ Behavior parity. */
  disableTxAfter73?: boolean
  /** Play a ding/dong audio cue when the dial crosses your TX privileges (default on). */
  bandEdgeTones?: boolean
  /** WSJT-X "CW ID after 73" — key MYCALL in CW after the final over (default off). */
  cwIdAfter73?: boolean
  clearDxAfterLog?: boolean
  doubleClickSetsTx?: boolean
  /** Tune carrier auto-release (seconds). */
  tuneTimeoutSecs?: number
  /** Field Day event: 'arrlfd' (default when empty) | 'wfd'. */
  fdEvent?: string
  /** FD power multiplier tier: 5 QRP-battery, 2 <=100W, 1 >100W. */
  fdPowerMult?: number
  /** Claimed FD bonus ids (the checklist). */
  fdBonuses?: string[]
  /** N3FJP real-time push (club master log). Empty host = off. */
  n3fjpHost?: string
  n3fjpPort?: number
  /** DXKeeper (DXLab) TCP push. Empty host = off. */
  dxkeeperHost?: string
  /** DXLab BASE port as shown in DXKeeper's own config panel; it listens on base + 1. */
  dxkeeperBasePort?: number
  dxkeeperUploads?: boolean
  /** Field Day push uses the contest-correct ENTER sequence (N3FJP scores it) vs ADDDIRECT. Default on. */
  n3fjpUseEnter?: boolean
  /** Report this position's band to N3FJP (no CAT) for the club Network Status Display board. Default off. */
  n3fjpReportBand?: boolean
  /** Auto-forward EVERY logged QSO to N3FJP ACLog (not just Field Day). Uses the host/port above. */
  n3fjpUpload?: boolean
  /** Cloudlog / Wavelog self-hosted logbook: base URL, station-profile id, instance API key. */
  cloudlogUrl?: string
  cloudlogStationId?: string
  /** Auto-forward each logged QSO to the Cloudlog/Wavelog instance above. */
  cloudlogUpload?: boolean
  /** N1MM contact broadcast target ("host" or "host:port"). Empty = off. */
  n1mmAddr?: string
  /** DXpedition special op: 'none' | 'hound' | 'superhound' (SuperFox hound). */
  specialOp?: 'none' | 'hound' | 'superhound'
  /** WSJT-X Split Operation: keep TX audio 1500-2000 Hz via dial shifts. */
  splitMode?: 'none' | 'rig' | 'fakeit'
  /** Operator overrides of the working-frequency table (empty = stock). */
  workingFrequencies?: { band: string; mode: string; mhz: number }[]
  /** FT8/FT4 decode depth: 1=Fast 2=Normal 3=Deep (stock Deep). */
  decodeDepth?: number
  /** Decoder passband (Hz) — WSJT-X F Low / F High. */
  decodeFLowHz?: number
  decodeFHighHz?: number
  // --- coordinated QSY ("move together") — separate, opt-in, off by default ---
  /** Master opt-in for coordinated QSY (announced-in-the-clear roaming). */
  qsyEnabled: boolean
  /** Band-plan channel tokens the initiator round-robins through when hopping. */
  qsySet: string[]
  /** Announce cadence: the initiator hops every this-many of its TX overs. */
  qsyCadence: number
  /** Opt-in (off by default): at ISS AOS auto-tune 145.800 FM + arm SSTV RX, and
   * restore the dial at LOS. Every rig-touching action is gated on this. */
  issSstvAutoArm: boolean
  /** Alert (beep + flash) when a decode is directed at my callsign. */
  alertMyCall: boolean
  /** Alert when any station is calling CQ. */
  alertCq: boolean
  /** Alert when a station not heard before this session appears. */
  alertNew: boolean
  /** Band scope for new-DXCC alerts: 'off' | 'hf' | 'vhf' | 'all' (alertNew stays the master). */
  alertDxccBands: string
  /** Band scope for plain new-grid alerts. Default 'vhf' — grid chasing is VHF-centric. */
  alertGridBands: string
  /** Band scope for the rare/ultra 💎 grid alerts (separate so HF-quiet keeps the gems). */
  alertRareGridBands: string
  /** Mouse-wheel tuning sensitivity (1.0 = stock; <1 less sensitive, >1 more). */
  wheelTuneSensitivity?: number
  /** Screen-reader speech for arriving decodes: 'off' | 'needed' (alerts only)
   * | 'all' (adds a per-batch summary with CQ callers). Silent without a
   * screen reader running. */
  announceVerbosity?: string
  /** Earcon on TX key/unkey (eyes-free TX pill). Off by default. */
  soundTxState?: boolean
  /** Soft tick when a decode batch lands. Off by default. */
  soundDecodeTick?: boolean
  // --- Auto-CQ caller selection (W1.4) ---
  /** When several stations answer your CQ, which to work first:
   *  'first' (stock), 'strongest', 'farthest', or 'cq_first'. */
  bestCaller: string
  /** Ignore answering stations weaker than this SNR (dB) when picking. null = no floor. */
  bestCallerMinSnr: number | null
  // --- Wanted watch list / alert filters (W1.5) ---
  /** Watch list: exact calls or trailing-* wildcard prefixes that raise a loud alert. */
  wantedCalls: string[]
  // --- confirmations (LoTW) ---
  /** LoTW account username (often, but not always, the callsign). The password is
   *  NOT here — it lives in the OS keychain (set via setLotwPassword). */
  lotwUsername: string
  /** Incremental-sync high-water cursor (APP_LoTW_LASTQSL). Managed by the app;
   *  not user-edited. Empty = next sync is a full pull. */
  lotwLastQsl: string
  /** LoTW upload Station Location name (the TQSL -l arg). Non-secret. Empty =
   *  upload not configured. */
  lotwStationLocation: string
  /** Sign LoTW uploads from the location embedded in the ADIF (STATION_CALLSIGN /
   *  MY_GRIDSQUARE) instead of a named TQSL Station Location — for travelers. */
  lotwUseAdifLocation: boolean
  /** Optional path to the tqsl binary (overrides auto-detect). */
  tqslPath: string
  /** eQSL account username (callsign or login). Password is in the OS keychain
   *  (set via setEqslPassword). */
  eqslUsername: string
  /** eQSL incremental-sync cursor (YYYYMMDDHHMM). Managed by the app; not
   *  user-edited. Empty = next sync is a full pull. */
  eqslLastSync: string
  /** QRZ.com account username for callsign lookup. Password is in the OS keychain
   *  (set via setQrzPassword); the session key is cached in memory only. */
  qrzUsername: string
  /** HamQTH.com account username — the FREE fallback for callsign lookup, used when
   *  QRZ isn't configured or has no match. Password is in the OS keychain (set via
   *  setHamqthPassword); the session id is cached in memory only. */
  hamqthUsername: string
  /** Auto-upload each logged QSO to the QRZ.com logbook. Needs the QRZ Logbook API
   *  key in the keychain (distinct from the lookup password). */
  qrzLogbookUpload: boolean
  /** ClubLog account email (not a callsign); app-password is in the keychain. */
  clublogEmail: string
  /** ClubLog logbook callsign to upload into (empty → your callsign). */
  clublogCallsign: string
  /** ClubLog developer/app API key (non-secret; never committed). Empty → a
   *  build-time default if the installer baked one in. */
  clublogApiKey: string
  /** Auto-upload each logged QSO to ClubLog (realtime push). */
  clublogUpload: boolean
  /** Auto-upload each logged QSO to eQSL.cc (ImportADIF). eQSL username is
   *  `eqslUsername`; the password is in the keychain. */
  eqslUpload: boolean
  /** Auto-upload each logged QSO to HRDLog.net (the online logging/awards site,
   *  NOT the HRD Logbook UDP push). Station callsign is `mycall`; the upload code
   *  is in the keychain. Not an ARRL confirmation source. */
  hrdlogUpload: boolean
  /** Watch near-region spots (not just your own paths) so opening detection can
   *  flag "a band is open around you" before you've worked anyone. */
  openingRegional: boolean
  /** Path-prediction engine: 'heuristic' (physics-lite default) or 'p533'
   * (native ITU-R P.533 — real circuit-reliability physics). */
  propEngine: string
  /** Save each RX period as a WAV: 'none' | 'all' | 'decodes' (WSJT-X Save menu). */
  saveWav?: string
  /** LoTW-user highlight window (days; default 365) — calls with an ARRL
   * activity-list upload within this window get the LoTW mark. */
  lotwMaxAgeDays?: number
  /** Antenna gains (dBi) for the P.533 link budget — 0 = isotropic/wire.
   * Honest v1: plain dB adders, no pattern modelling; heuristic ignores them. */
  antTxGainDbi?: number
  antRxGainDbi?: number
  /** Editable quick-reply macros, per context. */
  macros: {
    chat: string[]
    qso: string[]
    band: string[]
    /** LEGACY flat CW F-key list — kept for back-compat; migrated into `cwProfiles`
     * (a "Default" profile) on load. New reads/writes go through the profiles. */
    cw?: { key: string; label: string; text: string }[]
    /** Named CW cockpit F-key macro sets (one per operator/purpose). Each profile's
     * empty macro list = the cockpit's built-in defaults. */
    cwProfiles?: { name: string; macros: { key: string; label: string; text: string }[] }[]
    /** Index into `cwProfiles` of the active set. */
    activeCwProfile?: number
  }
  // --- dual-radio ---
  /** Configured radios (dual-radio). Migrated to a single profile for a one-radio station; the flat
   * rig/audio fields above mirror whichever is active. Absent on very old records. */
  radios?: RadioProfile[]
  /** The id of the active radio (the one the flat rig fields describe). */
  activeRadio?: number
  /** Peg-lock: band selection won't auto-switch the active radio when true. */
  radioPegged?: boolean
}

/** One radio's complete connection profile (dual-radio). Mirrors the Rust `RadioProfile`; the flat
 * `Settings` rig/audio fields are a mirror of whichever profile is active. The Settings roster edits
 * `name`/`bands` per radio; CAT/audio are edited via the flat form on the active radio. */
export interface RadioProfile {
  id: number
  name: string
  enabled: boolean
  pttMethod: string
  rigModel: number
  rigModelName: string
  serialPort: string
  baud: number
  rigConn: string
  rigAddr: string
  rigctldPort: number
  /** Native Icom CI-V: Nexus owns the CI-V serial port itself (real scope waveform +
   * instant dial tracking) instead of launching rigctld. Per-radio; default off. */
  icomNativeCat: boolean
  audioIn: string
  audioOut: string
  txLevel: number
  rxGain: number
  rotatorModel: number
  rotatorPort: string
  rotatorBaud: number
  rotatorHost: string
  rotctldPort: number
  /** Bands this radio covers (empty = all) — for auto band-routing (P4). */
  bands: string[]
  lastDialMhz: number
  lastBand: string
  lastSideband: string
  /** Native panadapter: "auto" | "none" | "flex" | "civ". */
  nativeScope: string
}

/** A compact per-radio summary for the multi-radio switcher (dual-radio). One per configured
 * radio; the active radio carries live band/frequency/S-meter, the others their last-known tune.
 * `radios` on the snapshot is empty or 1-element for a single-radio station (no switcher shown). */
export interface RadioSummary {
  id: number
  name: string
  band: string
  dialMhz: number
  sideband: string
  isActive: boolean
  /** Live CAT health for the active radio; null for a not-connected radio. */
  catOk: boolean | null
  /** Live S-meter (dB rel S9) for the active radio; null otherwise. */
  smeterDb: number | null
  transmitting: boolean
  /** Bands this radio covers (empty = all) — for auto-routing (P4) + a coverage hint. */
  bands: string[]
}

export interface AppSnapshot {
  mycall: string
  mygrid: string
  mode: OpMode
  radio: RadioStatus
  /** Multi-radio switcher summaries (dual-radio); empty/1-element ⇒ single-radio, no switcher. */
  radios?: RadioSummary[]
  /** The id of the active radio (matches one of `radios`). */
  activeRadioId?: number
  /** Peg-lock: band selection won't auto-switch the active radio when true. */
  radioPegged?: boolean
  /** AI CW decoder (beta): toggle state, status line, rolling stitched transcript. */
  aiCw: { enabled: boolean; status: string; text: string }
  link: LinkState
  /** Chat CQ run state (Tempo keep-calling loop): 'off' | 'calling' | 'paused'. */
  chatCq?: 'off' | 'calling' | 'paused'
  stations: Station[]
  conversations: Conversation[]
  activePeer: string | null
  qso: QsoStatus | null
  fieldDay: FieldDayStatus | null
  /** The most-recent RX slot's decoded signals (drives the live decode feed). */
  recentDecodes: DecodeRow[]
  /** JTAlert-style UDP callsign highlights for the decode panes. */
  highlights?: { call: string; bg?: string | null; fg?: string | null }[]
  /** Bumped by an inbound UDP Clear — panes erase on change. */
  clearTick?: number
  /** Bumped each time a spot is worked — App navigates to `workView`'s cockpit
   * on change (lets a pop-out window's click land the main window there). */
  workTick?: number
  /** The last worked spot's mode: 'digital' | 'phone' | 'cw'. */
  workView?: string | null
  /** The last worked spot's callsign — a pop-out window's click prefills the log Call from this. */
  workCall?: string | null
  /** Pending one-click POTA/SOTA hunt (next QSO with this call auto-tags). */
  hunt?: { program: string; reference: string; call: string } | null
  /** Coordinated-QSY status — present only while the opt-in feature is enabled. */
  qsy?: QsyStatus | null
  /** Session count of IR-HARQ rescues (decodes recovered by combining
   * retransmissions). Drives the HARQ stats readout. */
  harqRescues: number
  /** A completed QSO awaiting confirm-before-log (WSJT-X "Prompt me to log
   * QSO"). Present only with promptToLog on; drives the confirm popup. */
  pendingLog?: LoggedQso | null
  /** Last connector auto-upload outcome (QRZ/ClubLog/eQSL) from the backend
   * upload funnel; uploadTick bumps per outcome so the UI toasts it. */
  uploadNote?: string | null
  uploadOk?: boolean
  uploadTick?: number
}

// ── Program section (radio programming): repeater search + channel projects ──
// DTO mirrors of crates/propagation/src/{repeaters,memchan}.rs (serde camelCase)
// and the src-tauri radioprog commands. Keys MUST match the Rust serde names.

/** One repeater from a directory search, normalized across sources. */
export interface RepeaterRecord {
  source: 'repeaterbook' | 'hearham'
  sourceId: string
  callsign: string
  /** Repeater output (you listen here), MHz. */
  outputMhz: number
  /** Repeater input (you transmit here), MHz. */
  inputMhz: number
  ctcssEncHz?: number | null
  ctcssDecHz?: number | null
  dcs?: number | null
  lat: number
  lon: number
  city: string
  county: string
  state: string
  fm: boolean
  dmr: boolean
  dstar: boolean
  fusion: boolean
  dmrColorCode?: number | null
  bandwidthKhz?: number | null
  operational: boolean
  openUse: boolean
  distanceKm: number
  bearingDeg: number
}

/** One search row: the directory record (display) + the ready-to-add channel
 * (derived in the tested Rust domain — never re-derived in TS). */
export interface RepeaterSearchRow {
  record: RepeaterRecord
  channel: ProgChannel
}

/** A repeater search response: source label + data age + rows (nearest first). */
export interface RepeaterSearchResult {
  source: 'repeaterbook' | 'hearham'
  fetchedUtc: number
  /** True when a fetch failed/rate-limited and stale cache was served. */
  stale: boolean
  rows: RepeaterSearchRow[]
}

/** One geocoding candidate (OSM Nominatim). */
export interface GeoCandidate {
  displayName: string
  lat: number
  lon: number
}

/** One memory channel (mirror of propagation::memchan::Channel). */
export interface ProgChannel {
  id: string
  name: string
  rxMhz: number
  duplex: 'simplex' | 'plus' | 'minus' | 'split'
  offsetMhz: number
  toneMode: 'none' | 'tone' | 'tsql' | 'dtcs'
  rtoneHz: number
  ctoneHz: number
  dtcsCode: number
  mode: 'fm' | 'nfm' | 'am' | 'dmr' | 'dstar' | 'fusion'
  comment: string
  dmrColorCode?: number | null
  dmrTimeslot?: number | null
  dmrTalkgroup?: number | null
  dstarRpt1?: string | null
  dstarRpt2?: string | null
  source?: { source: string; sourceId: string; callsign: string } | null
}

/** Where a programming project's repeaters were searched from. */
export interface ProgOrigin {
  kind: 'station' | 'grid' | 'city'
  grid: string
  label: string
  lat: number
  lon: number
}

/** One saved programming project (a curated channel list). */
export interface RadioProgProject {
  id: string
  name: string
  createdUtc: number
  updatedUtc: number
  origin: ProgOrigin
  radiusKm: number
  channels: ProgChannel[]
}
