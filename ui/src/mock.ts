// Mock data + live-demo driver.
//
// Generates a realistic AppSnapshot and "runs" it: a setInterval advances the
// waterfall, toggles Tx/Rx, counts down the next slot, and occasionally injects
// an incoming message and a "peer is sending" indicator — so `npm run dev` shows
// a populated, lively demo without the radio stack attached.
//
// It also models the QSO sequencer and the Field Day workspace: setMode() seeds
// believable demo data for the requested mode and the live engine advances the
// QSO sequencer state and grows the Field Day log + score over time.

import type {
  AppSnapshot,
  Activation,
  AudioDevices,
  BandChannel,
  ChatMessage,
  DetectedRig,
  OtaSpot,
  Conversation,
  DecodeRow,
  FieldDayQso,
  FieldDayStatus,
  ImportStats,
  LoggedQso,
  LotwOrphan,
  LotwSyncResult,
  ModeRequest,
  NeedAlert,
  VoiceMessage,
  OpMode,
  QsoStatus,
  QsyStatus,
  Settings,
  Spectrum,
  PropagationSnapshot,
  SourceKind,
  Station,
  Tier,
  AwardSummary,
  JourneySummary,
  DiagnosticsReport,
} from './types'

/** A deterministic Propagation nowcast for the demo (mirrors the Rust
 *  `propagation::demo()` scene: a 6 m Es opening + a 20 m run to Europe + an
 *  active, needed DXpedition). Lets the Propagation section render without feeds. */
/** A plausible 24-h likelihood curve (Gaussian bump around a UTC peak hour). */
function hours(peakHour: number, peak: number, width: number): number[] {
  return Array.from({ length: 24 }, (_, h) => {
    let d = Math.abs(h - peakHour)
    d = Math.min(d, 24 - d) // wrap across midnight
    return Math.round(peak * Math.exp(-(d * d) / (2 * width * width)) * 100) / 100
  })
}

export function demoPropagation(): PropagationSnapshot {
  const band = (
    b: string,
    tier: PropagationSnapshot['advisory']['bands'][number]['tier'],
    score: number,
    nHearMe: number,
    nIHear: number,
    region: string | null,
    octant: string,
    bearingDeg: number,
    bidirectional: boolean,
    confidence: PropagationSnapshot['advisory']['bands'][number]['confidence'],
    reason: string,
  ) => ({
    band: b,
    tier,
    score,
    nHearMe,
    nIHear,
    bestRegion: region
      ? { region, octant, bearingDeg, stations: Math.max(nHearMe, nIHear), bidirectional }
      : null,
    confidence,
    reason,
  })
  const now = Math.floor(Date.now() / 1000)
  return {
    advisory: {
      headline:
        '⚡ 6M IS OPEN — point E at North America (8 hear you, you hear 8). Strong.',
      banners: [],
      bands: [
        band('6m', 'Active', 0.82, 8, 8, 'North America', 'E', 95, true, 'Strong', '8 stations'),
        band('20m', 'Active', 0.71, 14, 6, 'Europe', 'NE', 48, true, 'Strong', '14 stations'),
        band('40m', 'Moderate', 0.34, 3, 1, 'North America', 'E', 92, false, 'Likely', '3 stations'),
        band('17m', 'Moderate', 0.28, 4, 2, 'Europe', 'NE', 50, true, 'Likely', '4 stations'),
        band('15m', 'Quiet', 0.12, 1, 0, 'South America', 'SE', 150, false, 'Marginal', '1 station'),
        band('30m', 'Quiet', 0.1, 1, 0, null, '', 0, false, 'Marginal', '1 station'),
        band('10m', 'Quiet', 0.08, 1, 0, null, '', 0, false, 'Marginal', 'building — flux 142'),
        band('80m', 'Quiet', 0.06, 1, 0, null, '', 0, false, 'Marginal', 'daytime — D-layer'),
        band('12m', 'Closed', 0.0, 0, 0, null, '', 0, false, 'Marginal', 'no activity heard'),
        band('160m', 'Closed', 0.0, 0, 0, null, '', 0, false, 'Marginal', 'daytime — D-layer'),
        band('4m', 'Closed', 0.0, 0, 0, null, '', 0, false, 'Marginal', 'no activity heard'),
        band('2m', 'Closed', 0.0, 0, 0, null, '', 0, false, 'Marginal', 'no activity heard'),
      ],
    },
    openings: [
      {
        band: '6m',
        mode: 'Sporadic-E',
        octant: 'E',
        bearingDeg: 95,
        maxKm: 1980,
        probability: 0.92,
        stations: 8,
        confidence: 'Strong',
        confidenceScore: 0.92,
        reciprocalPairs: 6,
        anomalyZ: 7.4,
        onsetSecs: 540,
        isNew: false,
        note: 'High-MUF Es — watch 4 m / 2 m next',
      },
    ],
    dxpeditions: {
      workableNow: [
        {
          call: 'C91RU', entity: 'Mozambique', need: 'Atno', band: '40m',
          bearingDeg: 95, octant: 'E', distanceKm: 13800, status: 'WorkNow',
          likelihood: 'Good', likelihoodScore: 0.62, liveConfirmed: false,
          howToCall: 'Enable Hound mode; call 1000–4000 Hz', windowHint: 'open now', priority: 462, modes: ['FT8', 'CW', 'SSB'],
        },
        {
          call: 'C91RU', entity: 'Mozambique', need: 'Atno', band: '20m',
          bearingDeg: 95, octant: 'E', distanceKm: 13800, status: 'WorkNow',
          likelihood: 'Fair', likelihoodScore: 0.41, liveConfirmed: true,
          howToCall: 'Enable Hound mode; call 1000–4000 Hz', windowHint: 'open now — live spots', priority: 450, modes: ['FT8', 'CW', 'SSB'],
        },
      ],
      active: ['C91RU'],
      upcoming: [
        {
          call: '5H3DX', entity: 'Tanzania', region: 'Africa',
          startUnix: now + 86400 * 2, endUnix: now + 86400 * 12,
          bands: ['40m', '20m', '17m'], modes: ['CW', 'FT8', 'FT4'],
          octant: 'E', bearingDeg: 88, distanceKm: 13200, best: '40m Excellent 0000–0700Z',
          outlook: [
            { band: '40m', workability: 'Excellent', score: 0.86, window: '0000–0700Z', hourly: hours(3, 0.86, 5) },
            { band: '17m', workability: 'Good', score: 0.58, window: '1000–1600Z', hourly: hours(13, 0.58, 4) },
            { band: '20m', workability: 'Fair', score: 0.46, window: '1100–1500Z', hourly: hours(13, 0.46, 3) },
          ],
        },
        {
          call: 'VP8XYZ', entity: 'South Georgia', region: 'South America',
          startUnix: now + 86400 * 5, endUnix: now + 86400 * 18,
          bands: ['20m', '15m', '6m'], modes: ['CW', 'FT8'],
          octant: 'SE', bearingDeg: 160, distanceKm: 13900, best: '20m Good 1300–1700Z',
          outlook: [
            { band: '20m', workability: 'Good', score: 0.64, window: '1300–1700Z', hourly: hours(15, 0.64, 4) },
            { band: '40m', workability: 'Fair', score: 0.48, window: '2300–0700Z', hourly: hours(3, 0.48, 5) },
            { band: '15m', workability: 'Fair', score: 0.42, window: '1400–1600Z', hourly: hours(15, 0.42, 3) },
          ],
        },
      ],
    },
    spaceWx: { sfi: 142, kp: 3, aIndex: 9, xrayClass: 'B-class', flare: false },
    source: 'demo',
    asOf: now,
  }
}

const WATERFALL_BINS = 120
const SLOT_MS = 15000 // FT8-style 15 s slot; FT1 fast tier is shorter but this drives the demo clock

/** Synthetic peer key for the open band-activity / broadcast feed. */
const BROADCAST_PEER = '*'

// ---------------------------------------------------------------------------
// Seed snapshot
// ---------------------------------------------------------------------------

const MYCALL = 'KD9TAW'
const MYGRID = 'EN52'

const stations: Station[] = [
  { call: 'K2DEF', grid: 'FN20', snr: -9, lastHeardSlot: 0, heardCount: 14, presence: 'active', worked: true, country: 'United States' },
  { call: 'N0GHI', grid: 'EM48', snr: -14, lastHeardSlot: -1, heardCount: 6, presence: 'active', worked: false, country: 'United States' },
  { call: 'VE3JKL', grid: 'FN03', snr: -18, lastHeardSlot: -4, heardCount: 3, presence: 'idle', worked: false, country: 'Canada' },
  { call: 'KD8MNO', grid: 'EN82', snr: -21, lastHeardSlot: -11, heardCount: 2, presence: 'idle', worked: true, country: 'United States' },
  { call: 'W6PQR', grid: 'CM87', snr: -7, lastHeardSlot: -2, heardCount: 9, presence: 'active', worked: false, country: 'United States' },
  { call: 'AA1STU', grid: null, snr: -24, lastHeardSlot: -38, heardCount: 1, presence: 'stale', worked: false, country: 'United States' },
]

function msg(p: Partial<ChatMessage> & Pick<ChatMessage, 'text' | 'slot' | 'outbound'>): ChatMessage {
  return {
    from: p.outbound ? MYCALL : (p.from ?? null),
    to: p.outbound ? (p.to ?? null) : (p.to ?? MYCALL),
    text: p.text,
    slot: p.slot,
    directedToMe: p.directedToMe ?? !p.outbound,
    outbound: p.outbound,
    snr: p.snr ?? null,
    freqHz: p.freqHz ?? null,
    dtSec: p.dtSec ?? null,
    tier: p.tier ?? 'FT1',
  }
}

/** Build a broadcast (open, to=null) message for the band feed. */
function broadcastMsg(
  p: Partial<ChatMessage> & Pick<ChatMessage, 'text' | 'slot' | 'outbound'>,
): ChatMessage {
  return {
    from: p.outbound ? MYCALL : (p.from ?? null),
    to: null,
    text: p.text,
    slot: p.slot,
    directedToMe: false,
    outbound: p.outbound,
    snr: p.snr ?? null,
    freqHz: p.freqHz ?? null,
    dtSec: p.dtSec ?? null,
    tier: p.tier ?? 'FT1',
  }
}

const conversations: Conversation[] = [
  {
    peer: 'K2DEF',
    messages: [
      msg({ from: 'K2DEF', text: 'CQ CQ DE K2DEF FN20', slot: -8, outbound: false, directedToMe: false, snr: -9, freqHz: 1500, dtSec: 0.1, tier: 'FT1' }),
      msg({ to: 'K2DEF', text: 'K2DEF KD9TAW EN52', slot: -7, outbound: true, snr: null, freqHz: 1500, dtSec: 0.0, tier: 'FT1' }),
      msg({ from: 'K2DEF', text: 'KD9TAW K2DEF R-09', slot: -6, outbound: false, snr: -9, freqHz: 1500, dtSec: 0.1, tier: 'FT1' }),
      msg({ to: 'K2DEF', text: 'R RR-11', slot: -5, outbound: true, freqHz: 1500, dtSec: 0.0, tier: 'FT1' }),
      msg({ from: 'K2DEF', text: 'Name here is Dave, running 5W to a dipole', slot: -4, outbound: false, snr: -10, freqHz: 1502, dtSec: 0.2, tier: 'FT1' }),
      msg({ to: 'K2DEF', text: 'Nice! Sam in Wisconsin. Solid copy on the fast tier.', slot: -3, outbound: true, freqHz: 1500, dtSec: 0.0, tier: 'FT1' }),
      msg({ from: 'K2DEF', text: 'QSL. How is the band treating you tonight?', slot: -1, outbound: false, snr: -8, freqHz: 1501, dtSec: 0.1, tier: 'FT1' }),
    ],
  },
  {
    peer: 'W6PQR',
    messages: [
      msg({ from: 'W6PQR', text: 'KD9TAW de W6PQR — you around?', slot: -12, outbound: false, snr: -7, freqHz: 1320, dtSec: 0.0, tier: 'FT1' }),
      msg({ to: 'W6PQR', text: '73 for now, will catch you on the net', slot: -10, outbound: true, freqHz: 1320, dtSec: 0.0, tier: 'FT1' }),
    ],
  },
  {
    peer: 'N0GHI',
    messages: [
      msg({ from: 'N0GHI', text: 'Field Day exchange? 3A EM48', slot: -2, outbound: false, snr: -14, freqHz: 1740, dtSec: -0.1, tier: 'DX1' }),
    ],
  },
  {
    // Open band-activity / broadcast feed (peer "*", all messages to = null).
    peer: BROADCAST_PEER,
    messages: [
      broadcastMsg({ from: 'K2DEF', text: 'CQ CQ DE K2DEF FN20 — anyone on the fast tier?', slot: -9, outbound: false, snr: -9, freqHz: 1500, dtSec: 0.1, tier: 'FT1' }),
      broadcastMsg({ from: 'W6PQR', text: 'Net starting at the top of the hour on 1320 Hz', slot: -6, outbound: false, snr: -7, freqHz: 1320, dtSec: 0.0, tier: 'FT1' }),
      broadcastMsg({ to: null, text: 'KD9TAW EN52 testing turbo-eq, pse report', slot: -4, outbound: true, freqHz: 1500, dtSec: 0.0, tier: 'FT1' }),
      broadcastMsg({ from: 'N0GHI', text: 'Field Day this weekend! 3A EM48 will be active', slot: -2, outbound: false, snr: -14, freqHz: 1740, dtSec: -0.1, tier: 'DX1' }),
    ],
  },
]

function freshSpectrum(): Spectrum {
  const row = new Array<number>(WATERFALL_BINS)
  for (let i = 0; i < WATERFALL_BINS; i++) {
    row[i] = 0.06 + Math.random() * 0.05 // noise floor
  }
  return { row }
}

// ---------------------------------------------------------------------------
// Live decode feed + general logbook seeds
// ---------------------------------------------------------------------------

function decode(p: Partial<DecodeRow> & Pick<DecodeRow, 'from' | 'message'>): DecodeRow {
  return {
    from: p.from,
    snr: p.snr ?? -15,
    dtSec: p.dtSec ?? 0.1,
    freqHz: p.freqHz ?? 1500,
    message: p.message,
    isCq: p.isCq ?? false,
    directedToMe: p.directedToMe ?? false,
    worked: p.worked ?? false,
    country: p.country ?? null,
    newDxcc: p.newDxcc ?? false,
    newGrid: p.newGrid ?? false,
    mine: p.mine ?? false,
    tier: p.tier ?? 'FT1',
    rv: p.rv ?? 0,
  }
}

// A realistic multi-mode RX slot: FT8 DX, an FT4 contest exchange, the FT1 fast
// tier (one HARQ rescue), and a DX1 robust call directed at me — so the mode
// chips show the full spread the native engine decodes.
const recentDecodes: DecodeRow[] = [
  decode({ from: 'F5RXL', message: 'CQ F5RXL IN94', snr: -3, dtSec: -0.1, freqHz: 1197, isCq: true, country: 'France', newDxcc: true, newGrid: true, tier: 'FT8' }),
  decode({ from: 'EA6EE', message: 'N1JFU EA6EE R-07', snr: -14, dtSec: 0.2, freqHz: 641, country: 'Balearic Is.', tier: 'FT8' }),
  decode({ from: 'HA0DU', message: 'K1JT HA0DU KN07', snr: -13, dtSec: 0.3, freqHz: 590, country: 'Hungary', worked: true, tier: 'FT8' }),
  decode({ from: 'N9OY', message: 'CQ RU N9OY EN43', snr: -3, dtSec: -0.2, freqHz: 1640, isCq: true, country: 'United States', newGrid: true, tier: 'FT4' }),
  decode({ from: 'W6PQR', message: 'CQ W6PQR CM87', snr: -7, dtSec: 0.0, freqHz: 1320, isCq: true, country: 'United States', tier: 'FT1' }),
  decode({ from: 'VE3JKL', message: 'VE3JKL W1XYZ R-12', snr: -18, dtSec: 0.2, freqHz: 980, country: 'Canada', tier: 'FT1', rv: 1 }),
  decode({ from: 'N0GHI', message: `${MYCALL} N0GHI -14`, snr: -14, dtSec: -0.1, freqHz: 1740, directedToMe: true, country: 'United States', tier: 'DX1' }),
  // Our own transmitted call (yellow own-TX row — the "I called them" chronology).
  decode({ from: MYCALL, message: `F5RXL ${MYCALL} ${MYGRID}`, snr: 0, dtSec: 0, freqHz: 1500, mine: true, tier: 'FT8' }),
]

const NOW = Math.floor(Date.now() / 1000)
const logbook: LoggedQso[] = [
  { call: 'K2DEF', grid: 'FN20', country: 'United States', band: '20m', freqMhz: 14.0905, mode: 'FT1', rstSent: '-9', rstRcvd: '-11', whenUnix: NOW - 3600, confirmed: false, awardConfirmed: false },
  { call: 'KD8MNO', grid: 'EN82', country: 'United States', band: '40m', freqMhz: 7.0445, mode: 'FT1', rstSent: '-15', rstRcvd: '-18', whenUnix: NOW - 7200, confirmed: true, awardConfirmed: true },
  // Confirmed only via eQSL → NOT award-eligible (shows the distinction).
  { call: 'W1AW', grid: 'FN31', country: 'United States', band: '20m', freqMhz: 14.0905, mode: 'DX1', rstSent: '-3', rstRcvd: '-5', whenUnix: NOW - 86400, confirmed: true, awardConfirmed: false },
]

// ---------------------------------------------------------------------------
// Settings (held in memory; seeds the snapshot's mycall/grid/radio)
// ---------------------------------------------------------------------------

function defaultSettings(): Settings {
  return {
    mycall: MYCALL,
    mygrid: MYGRID,
    opName: '',
    clusterHost: 'telnet.reversebeacon.net:7000',
    companionAddr: '127.0.0.1:2237',
    cwPitchHz: 600,
    band: '20m',
    dialMhz: 14.074,
    sideband: 'USB',
    fdClass: '1D',
    fdSection: 'WI',
    fdEvent: 'arrlfd',
    fdPowerMult: 1,
    fdBonuses: [],
    n3fjpHost: '',
    n3fjpPort: 1100,
    n1mmAddr: '',
    licenseClass: 'open',
    // rig control
    pttMethod: 'vox',
    rigModel: 0,
    rigModelName: '',
    serialPort: '',
    baud: 38400,
    rigctldPort: 4532,
    catBroker: false,
    catBrokerPort: 4532,
    // audio
    audioIn: '',
    audioOut: '',
    txLevel: 0.9,
    txWatchdogMin: 6,
    // timing & tuning
    txEven: true,
    rxOffsetHz: 1500,
    txOffsetHz: 1500,
    holdTxFreq: false,
    clockCheck: true,
    // WSJT-X parity: Split Operation + working-frequency overrides
    splitMode: 'none',
    specialOp: 'none',
    workingFrequencies: [],
    // Decoder settings (WSJT-X parity: depth + passband)
    decodeDepth: 3,
    decodeFLowHz: 200,
    decodeFHighHz: 2900,
    // network integrations
    wsjtxUdp: false,
    wsjtxUdpAddr: '127.0.0.1:2237',
    hrdLogging: false,
    hrdUdpAddr: '127.0.0.1:2333',
    pskreporter: false,
    // off = passive (hunt & pounce); the only auto-TX path when enabled
    beacon: false,
    // IR-HARQ on by default (RV1/RV2 combine + TX redundancy escalation)
    harqEnabled: true,
    // logbook & alerts
    autoLog: true,
    promptToLog: false,
    preferRrr: false,
    // coordinated QSY — separate, opt-in, off by default
    qsyEnabled: false,
    qsySet: ['20m', '40m', '30m'],
    qsyCadence: 6,
    alertMyCall: true,
    alertCq: false,
    alertNew: true,
    lotwUsername: '',
    lotwLastQsl: '',
    lotwStationLocation: '',
    tqslPath: '',
    eqslUsername: '',
    eqslLastSync: '',
    qrzUsername: '',
    qrzLogbookUpload: false,
    clublogEmail: '',
    clublogCallsign: '',
    clublogApiKey: '',
    clublogUpload: false,
    eqslUpload: false,
    openingRegional: true,
    macros: {
      chat: ['73', 'QSL', 'Name?', 'QTH?', 'CQ'],
      qso: ['R-09', 'RRR', 'RR73', '73'],
      band: ['CQ CQ', 'QRZ?', 'Net check-in', '73 to all'],
    },
  }
}

// Sample rig + port enumerations (the desktop app returns the real ones).
const RIG_MODELS: [number, string][] = [
  [1, 'Hamlib Dummy'],
  [2, 'NET rigctl'],
  [3073, 'Icom IC-7300'],
  [1042, 'Yaesu FT-991A'],
  [2014, 'Kenwood TS-590SG'],
]

const SERIAL_PORTS: string[] = ['COM3', 'COM5']

// Sample audio device enumeration (the desktop app returns the real ones).
const AUDIO_DEVICES: AudioDevices = {
  input: ['Default', 'USB Audio CODEC', 'Microphone'],
  output: ['Default', 'USB Audio CODEC', 'Speakers'],
}

// Band-plan presets (the desktop app returns the authoritative list).
const BAND_PLAN: BandChannel[] = [
  { band: '160m', group: 'HF', dialMhz: 1.838, mode: 'USB', label: '160m', note: 'Top band — best after dark, regional NVIS' },
  { band: '80m', group: 'HF', dialMhz: 3.5775, mode: 'USB', label: '80m', note: 'Nighttime regional / state nets' },
  { band: '40m', group: 'HF', dialMhz: 7.0445, mode: 'USB', label: '40m', note: 'Reliable day & night workhorse' },
  { band: '30m', group: 'HF', dialMhz: 10.1425, mode: 'USB', label: '30m', note: 'WARC band — digital only, quiet' },
  { band: '20m', group: 'HF', dialMhz: 14.0905, mode: 'USB', label: '20m', note: 'Daytime DX mainstay' },
  { band: '17m', group: 'HF', dialMhz: 18.1015, mode: 'USB', label: '17m', note: 'WARC band — uncrowded DX' },
  { band: '15m', group: 'HF', dialMhz: 21.0905, mode: 'USB', label: '15m', note: 'Daytime DX when sun is active' },
  { band: '12m', group: 'HF', dialMhz: 24.9165, mode: 'USB', label: '12m', note: 'WARC band — solar-peak DX' },
  { band: '10m', group: 'HF', dialMhz: 28.1, mode: 'USB', label: '10m', note: 'Sporadic-E & solar-peak openings' },
  { band: '6m', group: 'VHF', dialMhz: 50.345, mode: 'USB', label: '6m', note: 'The magic band — Es / meteor scatter' },
  { band: '2m', group: 'VHF', dialMhz: 144.235, mode: 'USB', label: '2m SSB', note: 'Weak-signal SSB calling area' },
  { band: '2m-fm', group: 'VHF', dialMhz: 145.56, mode: 'FM', label: '2m FM', note: 'FM simplex — local rag-chew' },
  { band: '1.25m-fm', group: 'VHF', dialMhz: 223.56, mode: 'FM', label: '1.25m FM', note: '222 MHz FM simplex' },
  { band: '1.25m', group: 'VHF', dialMhz: 222.13, mode: 'USB', label: '1.25m SSB', note: '222 MHz weak-signal' },
  { band: '70cm', group: 'UHF', dialMhz: 432.45, mode: 'USB', label: '70cm SSB', note: '432 MHz weak-signal calling area' },
  { band: '70cm-fm', group: 'UHF', dialMhz: 445.95, mode: 'FM', label: '70cm FM', note: '440 FM simplex' },
]

function baseSnapshot(settings: Settings): AppSnapshot {
  return {
    mycall: settings.mycall,
    mygrid: settings.mygrid,
    mode: 'chat',
    radio: {
      dialMhz: settings.dialMhz,
      band: settings.band,
      sideband: settings.sideband,
      transmitting: false,
      slot: 0,
      nextSlotMs: SLOT_MS,
      timeSyncOk: true,
      rxLevel: 0.55,
      txEnabled: true,
      txAllowed: true,
      tuning: false,
      txWatchdog: false,
      qsoRecording: false,
      catOk: null,
      catDetail: 'VOX — no CAT (demo)',
      audioError: null,
      txEven: true,
      rxOffsetHz: 1500,
      txOffsetHz: 1500,
      txLevel: 0.9,
      holdTxFreq: false,
      clockOffsetMs: null,
      source: 'native',
      sourceLabel: 'Native (FT1)',
    },
    link: {
      tier: 'FT8',
      snrDb: -9,
      dtSec: 0.1,
      freqHz: 1500,
      rv: 0,
      state: 'Solid',
      quality: 0.82,
    },
    stations: stations.map((s) => ({ ...s })),
    conversations: conversations.map((c) => ({ peer: c.peer, messages: [...c.messages] })),
    activePeer: 'K2DEF',
    qso: null,
    fieldDay: null,
    recentDecodes: recentDecodes.map((d) => ({ ...d })),
    qsy: qsyState.enabled ? { ...qsyState } : null,
    harqRescues: 2,
    pendingLog: null,
  }
}

// ---------------------------------------------------------------------------
// Coordinated-QSY demo state (drives the panel + status chip in the mock).
// ---------------------------------------------------------------------------
const qsyState: QsyStatus = {
  enabled: false,
  paused: false,
  role: 'initiator',
  partner: 'K2DEF',
  home: '20m',
  current: '20m',
  nextChannel: null,
  nextSlot: null,
  lostSync: false,
}

// ---------------------------------------------------------------------------
// QSO + Field Day demo seeds
// ---------------------------------------------------------------------------

// Sequencer states cycled through while a QSO is live, ham-aware ordering.
const QSO_RUN_STATES = ['Calling CQ', 'Replied', 'Sending Report', 'Roger Report', 'Logged']
const QSO_SP_STATES = ['Listening', 'Answering CQ', 'Awaiting Report', 'Sending Roger', 'Logged']

function seedQso(running: boolean): QsoStatus {
  return running
    ? { state: 'Calling CQ', dxcall: null, rxReport: null, running: true, txNow: 'CQ AB1CD EN52', stalled: false }
    : { state: 'Listening', dxcall: 'K2DEF', rxReport: -11, running: false, txNow: null, stalled: false }
}

const FD_BANDS = ['20m', '40m', '15m', '80m', '10m']
const FD_CLASSES = ['1D', '2A', '3A', '1E', '2F']
const FD_SECTIONS = ['IL', 'IN', 'MI', 'EN', 'WCF', 'STX', 'EWA', 'ORG', 'SDG', 'NLI']
const FD_CALL_POOL = [
  'K2DEF', 'N0GHI', 'VE3JKL', 'W6PQR', 'KD8MNO', 'AA1STU', 'W1AW',
  'NA4FD', 'K5XYZ', 'WB2ABC', 'KC9PDX', 'N7QRP',
]

function seedFieldDayLog(): FieldDayQso[] {
  return [
    { call: 'W1AW', class: '3A', section: 'CT', band: '20m' },
    { call: 'K5XYZ', class: '2A', section: 'STX', band: '40m' },
    { call: 'N7QRP', class: '1E', section: 'EWA', band: '15m' },
  ]
}

function uniqueSections(log: FieldDayQso[]): number {
  return new Set(log.map((q) => q.section)).size
}

// Field Day scoring (simplified): 2 points per digital contact.
function fdPoints(qsoCount: number): number {
  return qsoCount * 2
}

function seedFieldDay(settings: Settings, running: boolean): FieldDayStatus {
  const log = seedFieldDayLog()
  return {
    myClass: settings.fdClass,
    mySection: settings.fdSection,
    running,
    state: running ? 'Running — Calling CQ FD' : 'S&P — Hunting',
    qsoCount: log.length,
    sections: uniqueSections(log),
    points: fdPoints(log.length),
    log,
  }
}

function randomFieldDayQso(): FieldDayQso {
  const call = FD_CALL_POOL[Math.floor(Math.random() * FD_CALL_POOL.length)]
  return {
    call,
    class: FD_CLASSES[Math.floor(Math.random() * FD_CLASSES.length)],
    section: FD_SECTIONS[Math.floor(Math.random() * FD_SECTIONS.length)],
    band: FD_BANDS[Math.floor(Math.random() * FD_BANDS.length)],
  }
}

// ---------------------------------------------------------------------------
// Log export serializers (mock — the desktop app owns the real export)
// ---------------------------------------------------------------------------

/** Map a Field Day band string to an ADIF-style frequency in MHz. */
const FD_BAND_MHZ: Record<string, string> = {
  '80m': '3.573',
  '40m': '7.074',
  '20m': '14.074',
  '15m': '21.074',
  '10m': '28.074',
}

function toCabrillo(fd: FieldDayStatus, settings: Settings): string {
  const lines: string[] = [
    'START-OF-LOG: 3.0',
    'CONTEST: ARRL-FIELD-DAY',
    `CALLSIGN: ${settings.mycall}`,
    `CATEGORY: ${fd.myClass}`,
    `LOCATION: ${fd.mySection}`,
    `CLAIMED-SCORE: ${fd.points}`,
    'CREATED-BY: Nexus (mock export)',
  ]
  for (const q of fd.log) {
    const freqKhz = (Number(FD_BAND_MHZ[q.band] ?? '14.074') * 1000).toFixed(0)
    lines.push(
      `QSO: ${freqKhz} DG ---------- ---- ${settings.mycall} ${fd.myClass} ${fd.mySection} ${q.call} ${q.class} ${q.section}`,
    )
  }
  lines.push('END-OF-LOG:')
  return lines.join('\n')
}

function toAdif(fd: FieldDayStatus): string {
  const header = ['Nexus log export (mock)', '<ADIF_VER:5>3.1.0', '<PROGRAMID:5>Nexus', '<EOH>']
  const records = fd.log.map((q) => {
    const freq = FD_BAND_MHZ[q.band] ?? '14.074'
    const fields = [
      adifField('CALL', q.call),
      adifField('BAND', q.band),
      adifField('FREQ', freq),
      adifField('MODE', 'DATA'),
      adifField('CLASS', q.class),
      adifField('ARRL_SECT', q.section),
    ]
    return `${fields.join(' ')} <EOR>`
  })
  return [...header, ...records].join('\n')
}

function adifField(name: string, value: string): string {
  return `<${name}:${value.length}>${value}`
}

// ---------------------------------------------------------------------------
// Live demo engine
// ---------------------------------------------------------------------------

const INCOMING_LINES = [
  'Copy that, switching to the robust tier',
  'QRZ? lots of QSB here',
  'Temp dropping fast, woodstove going',
  'R-12 — pse confirm grid',
  'Heard you call AA1STU, no joy',
  'GL on Field Day! 73',
  'My turbo-eq is locking up nicely now',
  'QSP for KD8MNO: meet at the rally point',
]

// Open broadcasts other stations send to the whole band.
const BROADCAST_LINES = [
  'CQ CQ DE {call} {grid} — fast tier, who is around?',
  'Net check-ins welcome, {call} running the frequency',
  '{call} testing new antenna, signal reports appreciated',
  'WX here is rough, lots of QSB on the band',
  '{call} QRT for now, 73 to all',
  'Field Day prep underway, {call} will be 2A',
  'Anyone copy the DX on the low end? {call}',
]

const BROADCASTERS = ['K2DEF', 'N0GHI', 'W6PQR', 'VE3JKL', 'KD8MNO']

type Listener = (snap: AppSnapshot) => void

function modeFor(req: ModeRequest): OpMode {
  if (req === 'chat') return 'chat'
  return req.startsWith('qso') ? 'qso' : 'fieldDay'
}

class MockEngine {
  private settings = defaultSettings()
  private snap = baseSnapshot(this.settings)
  private listeners = new Set<Listener>()
  private timer: number | null = null
  private tick = 0
  private slotElapsedMs = 0
  /** peer -> remaining ticks of a "sending..." indicator (0 = none) */
  private typing = new Map<string, number>()
  /** index into the active QSO sequencer state list */
  private qsoStep = 0
  /** running vs S&P role for the QSO sequencer */
  private qsoRunning = true
  /** DX grid for the active directed call (operator-typed or from the roster) */
  private dxGrid: string | null = null
  /** general ADIF logbook (most-recent first) */
  private logbook: LoggedQso[] = logbook.map((q) => ({ ...q }))
  private activation: Activation = { program: null, reference: null, qsoCount: 0 }
  /** rolling counter so injected decode rows look fresh */
  private decodeSeq = 0

  getSnapshot(): AppSnapshot {
    return this.snap
  }

  getSettings(): Settings {
    return { ...this.settings }
  }

  setSettings(settings: Settings): AppSnapshot {
    this.settings = { ...settings }
    // Reflect operator + radio settings into the live snapshot without losing
    // current mode / conversations / sequencer state.
    this.snap = {
      ...this.snap,
      mycall: settings.mycall,
      mygrid: settings.mygrid,
      radio: {
        ...this.snap.radio,
        dialMhz: settings.dialMhz,
        band: settings.band,
        sideband: settings.sideband,
      },
      fieldDay: this.snap.fieldDay
        ? { ...this.snap.fieldDay, myClass: settings.fdClass, mySection: settings.fdSection }
        : null,
    }
    this.emit()
    return this.snap
  }

  getSerialPorts(): Promise<string[]> {
    return Promise.resolve([...SERIAL_PORTS])
  }

  getRigModels(): Promise<[number, string][]> {
    return Promise.resolve(RIG_MODELS.map((m) => [m[0], m[1]] as [number, string]))
  }

  getOtaSpots(program: string): Promise<OtaSpot[]> {
    const pota: OtaSpot[] = [
      { program: 'POTA', reference: 'K-1234', name: 'Acadia National Park', activator: 'K1ABC', freqKhz: 14074, mode: 'FT8', spotter: 'W9XYZ', comment: 'QRP', grid: 'FN44' },
      { program: 'POTA', reference: 'VE-0789', name: 'Banff National Park', activator: 'VA6DEF', freqKhz: 7035, mode: 'CW', spotter: 'RBN', comment: 'RBN 8 dB', grid: 'DO31' },
    ]
    const sota: OtaSpot[] = [
      { program: 'SOTA', reference: 'W7A/MN-001', name: 'Humphreys Peak, 3850m, 10 points', activator: 'K7SO', freqKhz: 14062, mode: 'CW', spotter: 'NA6N', comment: 'S2S welcome', grid: 'DM45' },
    ]
    return Promise.resolve(program.toUpperCase() === 'SOTA' ? sota : pota)
  }

  setActivation(program: string, reference: string): Promise<Activation> {
    this.activation = { program: program.toUpperCase(), reference: reference.toUpperCase(), qsoCount: 0 }
    return Promise.resolve({ ...this.activation })
  }

  clearActivation(): Promise<Activation> {
    this.activation = { program: null, reference: null, qsoCount: 0 }
    return Promise.resolve({ ...this.activation })
  }

  getActivation(): Promise<Activation> {
    return Promise.resolve({ ...this.activation })
  }

  detectRigs(): Promise<DetectedRig[]> {
    // Demo: a native-USB IC-705 (identified) on a CP210x bridge with its CODEC.
    return Promise.resolve([
      {
        portName: 'COM5',
        vid: 0x10c4,
        pid: 0xea60,
        product: 'IC-705',
        manufacturer: 'Icom Inc.',
        suggestedModel: 3085,
        suggestedModelName: 'Icom IC-705',
        chip: 'Silicon Labs CP210x',
        driverNote: 'Windows needs the Silicon Labs CP210x VCP driver — install it, then Retry.',
        driverUrl: 'https://www.silabs.com/developer-tools/usb-to-uart-bridge-vcp-drivers',
        driverBundled: false,
        suggestedAudio: 'Microphone (USB Audio CODEC)',
      },
    ])
  }

  getBandPlan(): Promise<BandChannel[]> {
    return Promise.resolve(BAND_PLAN.map((c) => ({ ...c })))
  }

  setFrequency(dialMhz: number, band: string, mode: string): AppSnapshot {
    this.snap = {
      ...this.snap,
      radio: { ...this.snap.radio, dialMhz, band, sideband: mode },
    }
    // keep the in-memory settings consistent so the Settings form re-opens here
    this.settings = { ...this.settings, dialMhz, band, sideband: mode }
    this.emit()
    return this.snap
  }

  getAudioDevices(): Promise<AudioDevices> {
    return Promise.resolve({
      input: [...AUDIO_DEVICES.input],
      output: [...AUDIO_DEVICES.output],
    })
  }

  setTxEnabled(enabled: boolean): AppSnapshot {
    this.snap = {
      ...this.snap,
      radio: {
        ...this.snap.radio,
        txEnabled: enabled,
        // enabling clears a tripped watchdog; disabling also stops any TX/tune
        txWatchdog: enabled ? false : this.snap.radio.txWatchdog,
        transmitting: enabled ? this.snap.radio.transmitting : false,
        tuning: enabled ? this.snap.radio.tuning : false,
      },
    }
    this.emit()
    return this.snap
  }

  setTxLevel(level: number): AppSnapshot {
    const clamped = Math.max(0, Math.min(1, level))
    this.snap = { ...this.snap, radio: { ...this.snap.radio, txLevel: clamped } }
    this.settings = { ...this.settings, txLevel: clamped }
    this.emit()
    return this.snap
  }

  setTune(on: boolean): AppSnapshot {
    // tuning forces TX enabled and reflects in the transmitting flag
    this.snap = {
      ...this.snap,
      radio: {
        ...this.snap.radio,
        tuning: on,
        transmitting: on || this.snap.radio.transmitting,
        txEnabled: on ? true : this.snap.radio.txEnabled,
        txWatchdog: on ? false : this.snap.radio.txWatchdog,
      },
    }
    this.emit()
    return this.snap
  }

  haltTx(): AppSnapshot {
    // emergency stop: drop TX + tune immediately (Monitor stays as-is)
    this.snap = {
      ...this.snap,
      radio: { ...this.snap.radio, transmitting: false, tuning: false },
    }
    this.emit()
    return this.snap
  }

  testCat(): Promise<{ ok: boolean; detail: string }> {
    // The live demo has no real rig; be honest about it.
    return Promise.resolve({
      ok: false,
      detail: 'Demo mode — no radio connected. Install Nexus and connect your rig to test CAT.',
    })
  }

  setTxEven(even: boolean): AppSnapshot {
    this.snap = { ...this.snap, radio: { ...this.snap.radio, txEven: even } }
    this.settings = { ...this.settings, txEven: even }
    this.emit()
    return this.snap
  }

  setRxOffset(hz: number): AppSnapshot {
    const rx = Math.max(200, Math.min(2900, Math.round(hz)))
    const tx = this.snap.radio.holdTxFreq ? this.snap.radio.txOffsetHz : rx
    this.snap = { ...this.snap, radio: { ...this.snap.radio, rxOffsetHz: rx, txOffsetHz: tx } }
    this.settings = { ...this.settings, rxOffsetHz: rx, txOffsetHz: tx }
    this.emit()
    return this.snap
  }

  setTxOffset(hz: number): AppSnapshot {
    const tx = Math.max(200, Math.min(2900, Math.round(hz)))
    this.snap = { ...this.snap, radio: { ...this.snap.radio, txOffsetHz: tx } }
    this.settings = { ...this.settings, txOffsetHz: tx }
    this.emit()
    return this.snap
  }

  setHoldTxFreq(on: boolean): AppSnapshot {
    this.snap = { ...this.snap, radio: { ...this.snap.radio, holdTxFreq: on } }
    this.settings = { ...this.settings, holdTxFreq: on }
    this.emit()
    return this.snap
  }

  // ----- coordinated QSY ("move together") — demo behavior ----------------
  private qsy: QsyStatus = { ...qsyState }

  private refreshQsy(): AppSnapshot {
    this.snap = { ...this.snap, qsy: this.qsy.enabled ? { ...this.qsy } : null }
    this.emit()
    return this.snap
  }

  qsySetEnabled(on: boolean): AppSnapshot {
    const home = this.settings.qsySet[0] ?? this.snap.radio.band
    this.qsy = {
      ...this.qsy,
      enabled: on,
      paused: false,
      home,
      current: home,
      nextChannel: null,
      nextSlot: null,
      lostSync: false,
      partner: this.snap.activePeer,
      role: this.snap.activePeer ? 'initiator' : 'idle',
    }
    return this.refreshQsy()
  }

  qsyConfigure(channels: string[], cadence: number): AppSnapshot {
    this.settings = { ...this.settings, qsySet: channels, qsyCadence: Math.max(1, cadence) }
    return this.refreshQsy()
  }

  qsyMoveNow(): AppSnapshot {
    if (this.qsy.enabled && !this.qsy.paused) {
      const set = this.settings.qsySet.filter((c) => c !== this.qsy.current)
      this.qsy = {
        ...this.qsy,
        nextChannel: set[0] ?? this.qsy.current,
        nextSlot: this.snap.radio.slot + 8,
        lostSync: false,
      }
    }
    return this.refreshQsy()
  }

  qsyPause(on: boolean): AppSnapshot {
    this.qsy = { ...this.qsy, paused: on }
    return this.refreshQsy()
  }

  qsyStop(): AppSnapshot {
    return this.qsySetEnabled(false)
  }

  setMode(req: ModeRequest): AppSnapshot {
    const mode = modeFor(req)
    let qso: QsoStatus | null = null
    let fieldDay: FieldDayStatus | null = null

    if (mode === 'qso') {
      this.qsoRunning = req === 'qso-run'
      this.qsoStep = 0
      qso = seedQso(this.qsoRunning)
    } else if (mode === 'fieldDay') {
      const running = req === 'fieldday-run'
      fieldDay = seedFieldDay(this.settings, running)
    }

    this.snap = { ...this.snap, mode, qso, fieldDay }
    this.emit()
    return this.snap
  }

  setTier(tier: Tier): AppSnapshot {
    const label =
      this.snap.radio.source === 'companion' ? 'WSJT-X UDP' : `Native (${tier})`
    this.snap = {
      ...this.snap,
      link: { ...this.snap.link, tier },
      radio: { ...this.snap.radio, sourceLabel: label },
    }
    this.emit()
    return this.snap
  }

  setSource(kind: SourceKind): AppSnapshot {
    const label = kind === 'companion' ? 'WSJT-X UDP' : `Native (${this.snap.link.tier})`
    this.snap = {
      ...this.snap,
      radio: { ...this.snap.radio, source: kind, sourceLabel: label },
    }
    this.emit()
    return this.snap
  }

  exportLog(format: 'cabrillo' | 'adif'): Promise<string> {
    const fd = this.snap.fieldDay
    if (!fd || fd.log.length === 0) {
      return Promise.reject(
        new Error('No Field Day log to export — start a Field Day session first'),
      )
    }
    return Promise.resolve(format === 'adif' ? toAdif(fd) : toCabrillo(fd, this.settings))
  }

  isTyping(peer: string): boolean {
    return (this.typing.get(peer) ?? 0) > 0
  }

  subscribe(fn: Listener): () => void {
    this.listeners.add(fn)
    this.ensureRunning()
    return () => {
      this.listeners.delete(fn)
      if (this.listeners.size === 0) this.stop()
    }
  }

  selectPeer(peer: string | null): void {
    this.snap = { ...this.snap, activePeer: peer }
    this.emit()
  }

  sendMessage(peer: string, text: string): void {
    const tier: Tier = this.snap.link.tier
    const outgoing = msg({
      to: peer,
      text,
      slot: this.snap.radio.slot,
      outbound: true,
      freqHz: this.snap.link.freqHz,
      dtSec: 0,
      tier,
    })
    this.appendMessage(peer, outgoing)
    // kick the radio into a brief TX state for the delivery animation
    this.snap = {
      ...this.snap,
      radio: { ...this.snap.radio, transmitting: true },
    }
    this.emit()
    // schedule a likely reply from that peer
    this.typing.set(peer, 3)
  }

  /** Send an open broadcast (to = null) into the "*" band feed. */
  broadcast(text: string): AppSnapshot {
    const outgoing = broadcastMsg({
      text,
      slot: this.snap.radio.slot,
      outbound: true,
      freqHz: this.snap.link.freqHz,
      dtSec: 0,
      tier: this.snap.link.tier,
    })
    this.appendMessage(BROADCAST_PEER, outgoing)
    this.snap = {
      ...this.snap,
      radio: { ...this.snap.radio, transmitting: true },
    }
    this.emit()
    return this.snap
  }

  /** Answer a station: enter QSO (S&P) mode targeting that DX call. */
  overrideNextTx(call: string, text: string, grid?: string): AppSnapshot {
    // Browser-dev stub of the WSJT-X Tx-slot click: target the station and
    // show `text` as the queued over.
    if (!this.snap.qso || this.snap.qso.dxcall !== call) this.callStation(call, grid)
    if (this.snap.qso) this.snap.qso.txNow = text
    return this.getSnapshot()
  }

  callStation(call: string, grid?: string, message?: string, snr?: number): AppSnapshot {
    this.qsoRunning = false
    this.qsoStep = 1
    const station = this.snap.stations.find((s) => s.call === call)
    this.dxGrid = grid?.trim().toUpperCase() || station?.grid || null
    const me = this.settings.mycall
    // Mirror the backend's WSJT-X double-click mapping: the next Tx is fixed by
    // what the DX last sent *to me*. Falls back to the grid (Tx1) for a CQ / a
    // message addressed to someone else / no message.
    const rpt = (snr ?? station?.snr ?? -13)
    const fmt = (n: number) => `${n >= 0 ? '+' : '-'}${String(Math.abs(n)).padStart(2, '0')}`
    let txNow = `${call} ${me} ${this.settings.mygrid}`
    let state = 'Answering CQ'
    const tok = (message ?? '').trim().toUpperCase().split(/\s+/)
    if (tok.length >= 3 && tok[0] === me.toUpperCase()) {
      const third = tok[2]
      if (/^R[-+]\d{1,2}$/.test(third)) {
        txNow = `${call} ${me} RR73`
        state = 'Confirming (RR73)'
      } else if (/^[-+]\d{1,2}$/.test(third)) {
        txNow = `${call} ${me} R${fmt(rpt)}`
        state = 'Rogering report'
      } else if (third === 'RRR' || third === 'RR73') {
        txNow = `${call} ${me} 73`
        state = 'Signing 73'
      } else {
        // a grid reply → send the report
        txNow = `${call} ${me} ${fmt(rpt)}`
        state = 'Sending report'
        if (/^[A-R]{2}\d{2}$/.test(third)) this.dxGrid = third
      }
    }
    this.snap = {
      ...this.snap,
      mode: 'qso',
      activePeer: call,
      radio: { ...this.snap.radio, txEnabled: true },
      qso: {
        state,
        dxcall: call,
        rxReport: station ? station.snr : -13,
        running: false,
        txNow,
        stalled: false,
      },
      fieldDay: null,
    }
    this.emit()
    return this.snap
  }

  /** Operator "Log QSO": log the active QSO's contact now. */
  logCurrentQso(): AppSnapshot {
    const dx = this.snap.qso?.dxcall
    if (dx) {
      const rec: LoggedQso = {
        call: dx,
        grid: this.dxGrid,
        country: null,
        state: null,
        band: this.settings.band,
        freqMhz: this.settings.dialMhz,
        mode: this.snap.link.tier,
        rstSent: '-7',
        rstRcvd: this.snap.qso?.rxReport != null ? String(this.snap.qso.rxReport) : null,
        whenUnix: Math.floor(Date.now() / 1000),
        confirmed: false,
        awardConfirmed: false,
        creditGranted: [],
        creditSubmitted: [],
      }
      this.logbook = [rec, ...this.logbook]
      const stations = this.snap.stations.map((s) => (s.call === dx ? { ...s, worked: true } : s))
      this.snap = { ...this.snap, stations }
      this.emit()
    }
    return this.snap
  }

  /** Confirm-and-log a QSO held by the prompt-to-log popup. */
  confirmPendingLog(record: LoggedQso): AppSnapshot {
    this.logbook = [record, ...this.logbook]
    const stations = this.snap.stations.map((s) =>
      s.call === record.call ? { ...s, worked: true } : s,
    )
    this.snap = { ...this.snap, stations, pendingLog: null }
    this.emit()
    return this.snap
  }

  /** Discard a held QSO without logging it. */
  discardPendingLog(): AppSnapshot {
    this.snap = { ...this.snap, pendingLog: null }
    this.emit()
    return this.snap
  }

  /** Switch the operating area (mock: set the area's tier + mode). */
  setArea(area: 'dx' | 'msg'): AppSnapshot {
    if (area === 'msg') {
      const tier = this.snap.link.tier === 'DX1' ? 'DX1' : 'FT1'
      this.snap = { ...this.snap, mode: 'chat', link: { ...this.snap.link, tier } }
    } else {
      const tier = this.snap.link.tier === 'FT4' ? 'FT4' : 'FT8'
      this.snap = { ...this.snap, mode: this.snap.mode === 'chat' ? 'qso' : this.snap.mode, link: { ...this.snap.link, tier } }
    }
    this.emit()
    return this.snap
  }

  /** Operator "Resend": re-arm the current QSO message (mock no-op beyond echo). */
  qsoResend(): AppSnapshot {
    if (this.snap.qso) {
      this.snap = { ...this.snap, qso: { ...this.snap.qso, stalled: false } }
      this.emit()
    }
    return this.snap
  }

  /** Operator in-QSO free text (Tx5): set it as the next message. */
  qsoFreetext(text: string): AppSnapshot {
    const t = text.trim()
    if (this.snap.qso && t) {
      const dx = this.snap.qso.dxcall
      const txNow = dx ? `${dx} ${this.settings.mycall} ${t}` : t
      this.snap = { ...this.snap, qso: { ...this.snap.qso, txNow, stalled: false } }
      this.emit()
    }
    return this.snap
  }

  /** Append a contact to the logbook and mark the station worked. */
  logQso(record: LoggedQso): AppSnapshot {
    this.logbook = [record, ...this.logbook]
    const stations = this.snap.stations.map((s) =>
      s.call === record.call ? { ...s, worked: true } : s,
    )
    this.snap = { ...this.snap, stations }
    this.emit()
    return this.snap
  }

  getLog(): Promise<LoggedQso[]> {
    return Promise.resolve(this.logbook.map((q) => ({ ...q })))
  }

  /** Edit entry `index`, preserving sync-derived state (mirrors the engine). */
  editQso(index: number, record: LoggedQso): AppSnapshot {
    if (index >= 0 && index < this.logbook.length) {
      const old = this.logbook[index]
      this.logbook = this.logbook.map((q, i) =>
        i === index
          ? { ...record, confirmed: old.confirmed, awardConfirmed: old.awardConfirmed, upload: old.upload }
          : q,
      )
    }
    this.emit()
    return this.snap
  }

  /** Delete entry `index`. */
  deleteQso(index: number): AppSnapshot {
    if (index >= 0 && index < this.logbook.length) {
      this.logbook = this.logbook.filter((_, i) => i !== index)
    }
    this.emit()
    return this.snap
  }

  purgeLog(): number {
    const n = this.logbook.length
    this.logbook = []
    this.emit()
    return n
  }

  // A mid-journey operator (a few dozen entities/states, some firsts), so the demo
  // Journey board reads as alive and aspirational.
  getJourney(): Promise<JourneySummary> {
    const first = (
      id: string,
      title: string,
      meaning: string,
      unlocked: boolean,
      detail: string | null = null,
    ) => ({ id, title, meaning, heritage: '', unlocked, whenUnix: unlocked ? 1_700_000_000 : null, detail })
    const ladder = (
      id: string,
      title: string,
      worked: number,
      confirmed: number,
      rungs: [string, number][],
      max: number,
    ) => {
      const r = rungs.map(([label, target], i) => ({
        label,
        target,
        tier: (['bronze', 'bronze', 'silver', 'silver', 'gold', 'platinum'][i] ?? 'gold') as
          | 'bronze'
          | 'silver'
          | 'gold'
          | 'platinum'
          | 'legendary',
      }))
      return {
        id,
        title,
        meaning: '',
        heritage: '',
        worked,
        confirmed,
        rungs: r,
        nextRung: r.find((x) => worked < x.target) ?? null,
        max,
      }
    }
    const cells = (n: number, total: number, label: (i: number) => string) =>
      Array.from({ length: total }, (_, i) => ({
        key: String(i),
        label: label(i),
        worked: i < n,
        confirmed: i < Math.floor(n * 0.8),
      }))
    return Promise.resolve({
      level: 6,
      xp: 5240,
      xpIntoLevel: 240,
      xpForLevel: 1750,
      totalQsos: 312,
      nextMilestone: {
        ladderId: 'was',
        title: 'States (toward WAS) — Forty States',
        current: 37,
        target: 40,
        remaining: 3,
      },
      firsts: [
        first('first-qso', 'First Contact', 'Your very first logged QSO.', true, 'W1AW'),
        first('first-dx', 'First DX', 'Your first foreign country.', true, 'Germany'),
        first('first-digital', 'First Digital', 'Your first FT8/FT4 contact.', true, 'EA3XYZ'),
        first('first-1000mi', 'First 1,000-Mile Contact', 'You reached 1,000+ miles.', true),
        first('first-5000mi', 'First 5,000-Mile Contact', 'You spanned 5,000+ miles.', false),
        first('first-cw', 'First CW', 'A contact in Morse code.', false),
        first('first-vhf', 'First VHF (6 m+)', 'Your first 6 m+ contact.', false),
        first('first-pota', 'First POTA Contact', 'Worked a park activator.', true, 'K-1234'),
      ],
      ladders: [
        ladder(
          'dxcc',
          'Countries (toward DXCC)',
          48,
          39,
          [
            ['First DX', 1],
            ['Five Countries', 5],
            ['Globetrotter', 10],
            ['Quarter Century', 25],
            ['Half Century', 50],
            ['DXCC', 100],
          ],
          100,
        ),
        ladder(
          'was',
          'States (toward WAS)',
          37,
          31,
          [
            ['First State', 1],
            ['Five States', 5],
            ['Ten States', 10],
            ['Twenty-Five', 25],
            ['Forty States', 40],
            ['WAS', 50],
          ],
          50,
        ),
        ladder(
          'wac',
          'Continents (toward WAC)',
          5,
          4,
          [
            ['First Continent', 1],
            ['Three Continents', 3],
            ['WAC', 6],
          ],
          6,
        ),
      ],
      collections: [
        {
          id: 'states',
          title: 'Worked All States',
          meaning: 'Fill in all 50.',
          cells: cells(37, 50, (i) => String(i)),
          worked: 37,
          total: 50,
        },
        {
          id: 'continents',
          title: 'Worked All Continents',
          meaning: 'Six continents.',
          cells: ['NA', 'SA', 'EU', 'AS', 'OC', 'AF'].map((c, i) => ({
            key: c,
            label: c,
            worked: i < 5,
            confirmed: i < 4,
          })),
          worked: 5,
          total: 6,
        },
      ],
      feats: [
        {
          id: 'mode-slam',
          title: 'Mode Slam',
          meaning: 'CW + Phone + Digital.',
          heritage: '',
          tier: 'silver',
          unlocked: false,
          current: 2,
          target: 3,
          unit: 'modes',
          detail: null,
          gated: false,
          gateHint: null,
        },
        {
          id: 'miles-per-watt',
          title: '1000 Miles-per-Watt',
          meaning: 'Cover 1,000 miles per watt.',
          heritage: '',
          tier: 'legendary',
          unlocked: false,
          current: 0,
          target: 1000,
          unit: 'mi/W',
          detail: null,
          gated: true,
          gateHint: 'Set your station power in Settings to unlock miles-per-watt.',
        },
      ],
      bests: [
        { id: 'longest', title: 'Longest distance', value: '7,420 mi', detail: 'ZL3ABC' },
        { id: 'best-snr', title: 'Strongest signal', value: '+19 dB', detail: 'W1AW' },
        { id: 'busiest-day', title: 'Most QSOs in a day', value: '34', detail: '2024-06-22' },
      ],
      streak: { enabled: true, weeks: 4, activeThisWeek: true },
    })
  }

  // A mid-level DXer's award progress (past the 100-entity DXCC milestone,
  // chasing the rest + Challenge band slots), so the demo dashboard is alive.
  getAwards(): Promise<AwardSummary> {
    return Promise.resolve({
      qsos: 1287,
      confirmedQsos: 1043,
      dxccWorked: 142,
      dxccConfirmed: 118,
      dxccCredited: 100,
      readyToSubmit: 18,
      slotsWorked: 487,
      slotsConfirmed: 392,
      bands: [
        { band: '80m', worked: 41, confirmed: 33 },
        { band: '40m', worked: 88, confirmed: 71 },
        { band: '30m', worked: 64, confirmed: 52 },
        { band: '20m', worked: 121, confirmed: 104 },
        { band: '17m', worked: 79, confirmed: 61 },
        { band: '15m', worked: 67, confirmed: 48 },
        { band: '12m', worked: 23, confirmed: 14 },
        { band: '10m', worked: 38, confirmed: 9 },
      ],
      modes: [
        { mode: 'CW', worked: 96, confirmed: 81 },
        { mode: 'Phone', worked: 73, confirmed: 58 },
        { mode: 'Digital', worked: 128, confirmed: 109 },
      ],
      needed: [
        { entity: 'Bouvet', bands: ['20m', '17m'] },
        { entity: 'Mozambique', bands: ['40m', '20m'] },
        { entity: 'Crozet Island', bands: ['20m'] },
        { entity: 'Scarborough Reef', bands: ['15m'] },
        { entity: 'North Korea', bands: ['20m'] },
      ],
      slotNeeded: [
        { entity: 'South Africa', bands: ['80m', '40m', '12m'] },
        { entity: 'Australia', bands: ['80m', '12m'] },
        { entity: 'Japan', bands: ['12m', '10m'] },
        { entity: 'Argentina', bands: ['10m'] },
        { entity: 'New Zealand', bands: ['12m'] },
      ],
      achievements: [
        { id: 'qso-1', title: 'First Contact', detail: 'Log your first QSO', category: 'QSOs', unlocked: true, current: 1287, target: 1, critical: true },
        { id: 'qso-10', title: 'Getting Going', detail: '10 QSOs in the log', category: 'QSOs', unlocked: true, current: 1287, target: 10, critical: false },
        { id: 'qso-100', title: 'Century', detail: '100 QSOs logged', category: 'QSOs', unlocked: true, current: 1287, target: 100, critical: false },
        { id: 'qso-1000', title: 'Worked the World', detail: '1,000 QSOs logged', category: 'QSOs', unlocked: true, current: 1287, target: 1000, critical: true },
        { id: 'dx-first', title: 'First DX', detail: 'Work your first DX entity', category: 'DXCC', unlocked: true, current: 142, target: 2, critical: true },
        { id: 'rare-1', title: 'DXpedition Contact', detail: 'Work a most-wanted DXCC entity', category: 'DXpeditions', unlocked: true, current: 7, target: 1, critical: true },
        { id: 'rare-5', title: 'DXpedition Hunter', detail: 'Work 5 most-wanted entities', category: 'DXpeditions', unlocked: true, current: 7, target: 5, critical: false },
        { id: 'dx-25', title: 'Globetrotter', detail: '25 entities worked', category: 'DXCC', unlocked: true, current: 142, target: 25, critical: false },
        { id: 'dx-50', title: 'Half-Century DX', detail: '50 entities worked', category: 'DXCC', unlocked: true, current: 142, target: 50, critical: false },
        { id: 'cfm-1', title: 'First Confirmation', detail: 'Confirm your first entity', category: 'DXCC', unlocked: true, current: 118, target: 1, critical: false },
        { id: 'dxcc-100', title: 'DXCC', detail: '100 confirmed entities — the DXCC award!', category: 'DXCC', unlocked: true, current: 118, target: 100, critical: true },
        { id: 'honor-roll', title: 'DXCC Honor Roll', detail: 'Confirm all but 9 of the current DXCC entities', category: 'DXCC', unlocked: false, current: 118, target: 331, critical: true },
        { id: 'honor-roll-1', title: '#1 Honor Roll', detail: 'Confirm every current DXCC entity — the top of the list', category: 'DXCC', unlocked: false, current: 118, target: 340, critical: true },
        { id: 'chal-100', title: 'Slot Collector', detail: '100 confirmed band slots', category: 'Challenge', unlocked: true, current: 392, target: 100, critical: false },
        { id: 'chal-500', title: 'Slot Hunter', detail: '500 confirmed band slots', category: 'Challenge', unlocked: false, current: 392, target: 500, critical: false },
        { id: 'chal-1000', title: 'DXCC Challenge', detail: '1,000 confirmed slots — the Challenge!', category: 'Challenge', unlocked: false, current: 392, target: 1000, critical: true },
        { id: 'waz-half', title: 'Zone Collector', detail: 'Confirm 20 CQ zones', category: 'WAZ', unlocked: true, current: 37, target: 20, critical: false },
        { id: 'waz-40', title: 'Worked All Zones', detail: 'Confirm all 40 CQ zones — the WAZ award!', category: 'WAZ', unlocked: false, current: 37, target: 40, critical: true },
        { id: 'was-half', title: 'Halfway to WAS', detail: 'Confirm 25 US states', category: 'WAS', unlocked: true, current: 46, target: 25, critical: false },
        { id: 'was-50', title: 'Worked All States', detail: 'Confirm all 50 US states — the WAS award!', category: 'WAS', unlocked: false, current: 46, target: 50, critical: true },
      ],
      fiveBandWorked: 78,
      fiveBandConfirmed: 64,
      wazWorked: 39,
      wazConfirmed: 37,
      honorRoll: {
        currentTotal: 340,
        confirmed: 118,
        threshold: 331,
        achieved: false,
        needed: 213,
        numberOne: false,
        numberOneNeeded: 222,
      },
      was: {
        worked: 50,
        confirmed: 46,
        needed: ['AK', 'HI', 'ND', 'WY'],
        fiveBandWorked: 28,
        fiveBandConfirmed: 19,
      },
      bandTargets: [
        { entity: 'Chad', bands: ['10m'] },
        { entity: 'Nepal', bands: ['80m'] },
        { entity: 'Mongolia', bands: ['80m', '10m'] },
        { entity: 'Bhutan', bands: ['15m', '10m'] },
      ],
    })
  }

  // A small sample so the Confirmations panel is alive in mock/browser mode.
  getConfirmationDiagnostics(): Promise<DiagnosticsReport> {
    return Promise.resolve({
      diagnoses: [
        {
          index: 5,
          award: 'DXCC/WAS',
          status: 'needsAction',
          reasons: [
            {
              code: 'r1',
              confidence: 'confident',
              explanation: 'EA7KW is logged but never uploaded to LoTW — upload it.',
              action: { kind: 'uploadToLotw' },
            },
          ],
        },
        {
          index: 6,
          award: 'DXCC/WAS',
          status: 'needsAction',
          reasons: [
            {
              code: 'r1',
              confidence: 'confident',
              explanation: 'I2ABC is logged but never uploaded to LoTW — upload it.',
              action: { kind: 'uploadToLotw' },
            },
          ],
        },
        {
          index: 7,
          award: 'DXCC/WAS',
          status: 'needsAction',
          reasons: [
            {
              code: 'r1',
              confidence: 'confident',
              explanation: 'F5XYZ is logged but never uploaded to LoTW — upload it.',
              action: { kind: 'uploadToLotw' },
            },
          ],
        },
        {
          index: 8,
          award: 'DXCC/WAS',
          status: 'needsAction',
          reasons: [
            {
              code: 'r9',
              confidence: 'confident',
              explanation: 'Your LoTW upload of VK3ABC bounced (invalid Station Location) — fix and re-upload.',
              action: { kind: 'reUpload', source: 'LoTW', detail: 'invalid Station Location' },
            },
          ],
        },
        {
          index: 9,
          award: 'DXCC/WAS',
          status: 'needsAction',
          reasons: [
            {
              code: 'r9',
              confidence: 'confident',
              explanation: 'LoTW rejected your certificate / Station Location for ZL2AB — fix it in TQSL, then re-upload.',
              action: { kind: 'reauthenticate', source: 'LoTW' },
            },
          ],
        },
        {
          index: 12,
          award: 'DXCC/WAS',
          status: 'confirmedWrongSource',
          reasons: [
            {
              code: 'r3',
              confidence: 'confident',
              explanation:
                'JA1XYZ is confirmed on a non-award source (eQSL/QRZ) only — that does NOT count for ARRL DXCC/WAS.',
              action: { kind: 'uploadToLotw' },
            },
          ],
        },
        {
          index: 20,
          award: 'DXCC/WAS',
          status: 'needsAction',
          reasons: [
            {
              code: 'r2',
              confidence: 'likely',
              explanation: "You're in LoTW for PY2XX — waiting on them to upload/confirm.",
              action: { kind: 'nudgePartner', call: 'PY2XX', source: 'LoTW' },
            },
          ],
        },
        {
          index: 47,
          award: 'DXCC/WAS',
          status: 'needsAction',
          reasons: [
            {
              code: 'r4a',
              confidence: 'confident',
              explanation: 'DL5ABC confirmed on a different band than your log — fix the band so it matches.',
              action: { kind: 'fixField', field: 'BAND', found: '20m', expected: '40m' },
            },
          ],
        },
        {
          index: 30,
          award: 'DXCC/WAS',
          status: 'needsAction',
          reasons: [
            {
              code: 'r9',
              confidence: 'confident',
              explanation: 'Your QRZ upload of OH2XX bounced (rejected ADIF) — fix and re-upload.',
              action: { kind: 'reUpload', source: 'QRZ', detail: 'rejected ADIF' },
            },
          ],
        },
        {
          index: 31,
          award: 'DXCC/WAS',
          status: 'needsAction',
          reasons: [
            {
              code: 'r1',
              confidence: 'confident',
              explanation: 'SP9ABC is logged but never uploaded to QRZ — upload it.',
              action: { kind: 'uploadToQrz' },
            },
          ],
        },
        {
          index: 32,
          award: 'DXCC/WAS',
          status: 'needsAction',
          reasons: [
            {
              code: 'r9',
              confidence: 'confident',
              explanation: 'ClubLog rejected your login for VK9XX — fix it in Settings, then re-upload.',
              action: { kind: 'reauthenticate', source: 'ClubLog' },
            },
          ],
        },
        {
          index: 88,
          award: 'DXCC/WAS',
          status: 'confirmed',
          reasons: [
            {
              code: 'r4d',
              confidence: 'confident',
              explanation: 'W7ABC is confirmed for DXCC but has no STATE — WAS can’t credit it. Set the state.',
              action: { kind: 'fixField', field: 'STATE', found: '', expected: 'the worked station’s US state' },
            },
          ],
        },
      ],
      // Ordered as the engine emits: count DESC, then kind ASC. R9 is split per source +
      // re-upload/re-auth so each bucket is homogeneous (the bulk button only ever fires
      // for an all-LoTW-(re)upload bucket).
      buckets: [
        { kind: 'Logged but never uploaded to LoTW', count: 3, qsoIndices: [5, 6, 7] },
        { kind: 'ClubLog login rejected — fix it in Settings', count: 1, qsoIndices: [32] },
        { kind: 'Confirmed elsewhere — not ARRL-eligible (get LoTW/paper)', count: 1, qsoIndices: [12] },
        { kind: 'Field mismatch blocking a confirmation', count: 1, qsoIndices: [47] },
        { kind: 'LoTW rejected your certificate — fix it in TQSL', count: 1, qsoIndices: [9] },
        { kind: 'LoTW upload bounced — fix & re-upload', count: 1, qsoIndices: [8] },
        { kind: 'Logged but never uploaded to QRZ', count: 1, qsoIndices: [31] },
        { kind: 'Missing STATE for WAS', count: 1, qsoIndices: [88] },
        { kind: 'QRZ upload bounced — fix & re-upload', count: 1, qsoIndices: [30] },
        { kind: 'Uploaded — waiting on the other operator', count: 1, qsoIndices: [20] },
      ],
      waitingOnPartner: 6,
      pendingLag: 9,
    })
  }

  importAdif(text: string): Promise<ImportStats> {
    // Demo-grade: pull callsigns out of the ADIF and add minimal rows so the
    // count + list reflect the import. The real merge/dedup is in the engine.
    const calls = [...text.matchAll(/<CALL:\d+>([A-Z0-9/]+)/gi)].map((m) => m[1].toUpperCase())
    const now = Math.floor(Date.now() / 1000)
    for (const call of calls) {
      this.logbook = [
        { call, grid: null, band: '20m', freqMhz: 14.074, mode: 'FT8', rstSent: null, rstRcvd: null, whenUnix: now, confirmed: false, awardConfirmed: false },
        ...this.logbook,
      ]
    }
    return Promise.resolve({ added: calls.length, skipped: 0, total: this.logbook.length })
  }

  syncLotwReport(text: string): Promise<LotwSyncResult> {
    // Demo-grade reconcile: confirm + DXCC-credit any logged call the report
    // names; report names with no logged QSO as orphans. NOTE: matches on
    // callsign only (lossy) — the real Rust reconcile keys on
    // (call, band, mode-class, UTC-day) with consume-once, so demo diff counts
    // won't exactly mirror production.
    const calls = [...text.matchAll(/<CALL:\d+>([A-Z0-9/]+)/gi)].map((m) => m[1].toUpperCase())
    let newlyConfirmed = 0
    let newlyConfirmedAny = 0
    let newlyCredited = 0
    let matched = 0
    const orphans: LotwOrphan[] = []
    for (const call of calls) {
      const hit = this.logbook.find((q) => q.call.toUpperCase() === call)
      if (hit) {
        matched++
        if (!hit.confirmed) {
          hit.confirmed = true
          newlyConfirmedAny++
        }
        if (!hit.awardConfirmed) {
          hit.awardConfirmed = true
          hit.confirmed = true
          newlyConfirmed++
        }
        if (!(hit.creditGranted ?? []).includes('DXCC')) {
          hit.creditGranted = [...(hit.creditGranted ?? []), 'DXCC']
          newlyCredited++
        }
      } else {
        orphans.push({
          call,
          band: '20m',
          mode: 'FT8',
          whenUnix: Math.floor(Date.now() / 1000),
          reason: `no logged QSO with ${call}`,
        })
      }
    }
    return Promise.resolve({
      matched,
      newlyConfirmed,
      newlyConfirmedAny,
      newlyCredited,
      newlySubmitted: 0,
      promoted: 0, // own-echo promotion runs only on the online sync, not a paste
      orphans,
    })
  }

  getNeedAlerts(): Promise<NeedAlert[]> {
    // Demo: a few ranked opportunities so the "Needs heard now" panel renders.
    // The trailing three share callsigns with the mock roster so the Stations
    // panel shows need-tier colouring (NEW/ZONE/BAND/CFM) on those cards.
    return Promise.resolve([
      { call: '3Y0J', entity: 'Bouvet', band: '20m', zone: 38, tags: ['NewEntity', 'NewZone'], priority: 100, headline: 'New one — Bouvet', mode: 'Digital', freqMhz: null },
      // Voice/CW needs from the cluster — carry an exact freq + a click-to-work cockpit.
      { call: 'VK0MQ', entity: 'Macquarie Is.', band: '20m', zone: 30, tags: ['NewEntity'], priority: 100, headline: 'New one — Macquarie Is.', mode: 'Phone', freqMhz: 14.255 },
      { call: 'ZL7DX', entity: 'Chatham Is.', band: '40m', zone: 32, tags: ['NewEntity', 'NewZone'], priority: 100, headline: 'New one — Chatham Is.', mode: 'CW', freqMhz: 7.018 },
      { call: 'UA9XYZ', entity: 'Asiatic Russia', band: '20m', zone: 17, tags: ['NewZone'], priority: 70, headline: 'New CQ zone 17 — Asiatic Russia', mode: 'Digital', freqMhz: null },
      { call: 'JA3ABC', entity: 'Japan', band: '40m', zone: 25, tags: ['NewBand'], priority: 50, headline: 'New band — Japan 40m', mode: 'Digital', freqMhz: null },
      { call: 'W6PQR', entity: 'United States', band: '20m', zone: 3, tags: ['NewBand'], priority: 50, headline: 'New band — United States 20m', mode: 'Digital', freqMhz: null },
      { call: 'VE3JKL', entity: 'Canada', band: '20m', zone: 4, tags: ['NewZone'], priority: 70, headline: 'New CQ zone 4 — Canada', mode: 'Digital', freqMhz: null },
      { call: 'N0GHI', entity: 'United States', band: '20m', zone: 4, tags: ['Confirm'], priority: 10, headline: 'Confirm — United States', mode: 'Digital', freqMhz: null },
    ])
  }

  setLicenseClass(_licenseClass: string): Promise<AppSnapshot> {
    return Promise.resolve(this.snap)
  }

  getLicensedBandPlan(): Promise<BandChannel[]> {
    // Demo: reuse the standard band plan (mock is Open → all bands available).
    return this.getBandPlan()
  }

  startQsoRecording(): Promise<AppSnapshot> {
    this.snap = { ...this.snap, radio: { ...this.snap.radio, qsoRecording: true } }
    return Promise.resolve(this.snap)
  }

  stopQsoRecording(): Promise<AppSnapshot> {
    this.snap = { ...this.snap, radio: { ...this.snap.radio, qsoRecording: false } }
    return Promise.resolve(this.snap)
  }

  getVoiceMessages(): Promise<VoiceMessage[]> {
    // Demo: the default casual set, with a couple pre-"recorded" so the keyer strip
    // shows playable buttons in the browser preview.
    return Promise.resolve([
      { slot: 1, label: 'CQ', file: 'demo/cq.wav' },
      { slot: 2, label: 'My Call', file: '' },
      { slot: 3, label: 'Report', file: '' },
      { slot: 4, label: 'QRZ?', file: '' },
      { slot: 5, label: '73', file: 'demo/73.wav' },
      { slot: 6, label: 'Again', file: '' },
    ])
  }

  // -- internals ----------------------------------------------------------

  private ensureRunning(): void {
    if (this.timer !== null) return
    this.timer = window.setInterval(() => this.advance(), 250)
  }

  private stop(): void {
    if (this.timer !== null) {
      window.clearInterval(this.timer)
      this.timer = null
    }
  }

  private advance(): void {
    this.tick++
    this.slotElapsedMs += 250

    // --- waterfall scroll (handled by the canvas via getSpectrum) ---
    // --- slot countdown / boundary ---
    let radio = { ...this.snap.radio }
    radio.nextSlotMs = Math.max(0, SLOT_MS - this.slotElapsedMs)

    let slotRolled = false
    if (this.slotElapsedMs >= SLOT_MS) {
      this.slotElapsedMs = 0
      radio.slot += 1
      radio.nextSlotMs = SLOT_MS
      slotRolled = true
      // alternate Rx/Tx flavor across slots; mostly Rx
      radio.transmitting = false
    }

    // brief auto-clear of a manual TX (after ~1.5 s)
    if (radio.transmitting && this.tick % 6 === 0) {
      radio.transmitting = false
    }

    // --- jitter the RX audio level so the meter looks alive ---
    // wander toward a comfortable ~0.6 with the odd peak; never while keyed
    {
      const target = radio.transmitting || radio.tuning ? 0.05 : 0.6
      const drift = (target - radio.rxLevel) * 0.15
      const noise = (Math.random() - 0.5) * 0.18
      const peak = !radio.transmitting && Math.random() < 0.04 ? 0.3 : 0
      radio.rxLevel = clamp01(radio.rxLevel + drift + noise + peak)
    }

    // --- gently wander the link telemetry so the pill feels alive ---
    let link = { ...this.snap.link }
    if (slotRolled) {
      const jitter = (Math.random() - 0.5) * 1.6
      link.snrDb = clampRound(link.snrDb + jitter, -24, -3)
      link.dtSec = Math.round((link.dtSec + (Math.random() - 0.5) * 0.1) * 10) / 10
      link.quality = clamp01(link.quality + (Math.random() - 0.5) * 0.08)
      link.state = link.quality > 0.6 ? 'Solid' : link.quality > 0.35 ? 'Marginal' : 'Weak'
      link.rv = link.quality > 0.6 ? 0 : link.quality > 0.35 ? 1 : 2
    }

    // --- presence / last-heard drift ---
    let stations = this.snap.stations
    if (slotRolled) {
      stations = stations.map((s) => agePresence(s, radio.slot))
    }

    // --- typing indicators decay; on expiry, deliver an incoming line ---
    let conversations = this.snap.conversations
    if (slotRolled && this.typing.size) {
      for (const [peer, ticks] of [...this.typing.entries()]) {
        const left = ticks - 1
        if (left <= 0) {
          this.typing.delete(peer)
          conversations = this.deliverIncoming(conversations, peer, radio.slot)
        } else {
          this.typing.set(peer, left)
        }
      }
    }

    // --- occasionally a station starts sending to us spontaneously ---
    if (slotRolled && Math.random() < 0.3) {
      const candidate = stations.find((s) => s.presence === 'active')
      if (candidate && !this.typing.has(candidate.call)) {
        this.typing.set(candidate.call, 1 + Math.floor(Math.random() * 2))
      }
    }

    // --- occasionally a station drops an open broadcast on the band ---
    if (slotRolled && Math.random() < 0.4) {
      conversations = this.deliverBroadcast(conversations, radio.slot)
    }

    // --- on each RX slot, refresh the live decode feed (keeps alerts alive) ---
    let recentDecodes = this.snap.recentDecodes
    if (slotRolled) {
      recentDecodes = this.rollDecodes(recentDecodes)
    }

    // --- advance the active mode's sequencer / scoreboard ---
    let qso = this.snap.qso
    let fieldDay = this.snap.fieldDay
    let pendingLog = this.snap.pendingLog ?? null
    if (slotRolled && this.snap.mode === 'qso' && qso) {
      qso = this.advanceQso(qso)
      // Prompt-to-log demo: when a contact reaches "Logged" and the operator
      // asked to confirm first, surface a pending record instead of silent log.
      if (qso.state === 'Logged' && this.settings.promptToLog && qso.dxcall && !pendingLog) {
        pendingLog = {
          call: qso.dxcall,
          grid: this.dxGrid,
          state: null,
          band: this.settings.band,
          freqMhz: this.settings.dialMhz,
          mode: this.snap.link.tier,
          rstSent: '-7',
          rstRcvd: qso.rxReport != null ? String(qso.rxReport) : null,
          whenUnix: Math.floor(Date.now() / 1000),
          confirmed: false,
          awardConfirmed: false,
          creditGranted: [],
          creditSubmitted: [],
        }
      }
    }
    if (slotRolled && this.snap.mode === 'fieldDay' && fieldDay) {
      fieldDay = this.advanceFieldDay(fieldDay)
    }

    this.snap = { ...this.snap, radio, link, stations, conversations, qso, fieldDay, recentDecodes, pendingLog }
    this.emit()
  }

  /** Step the QSO sequencer through its states and cycle to a fresh contact. */
  private advanceQso(qso: QsoStatus): QsoStatus {
    const states = this.qsoRunning ? QSO_RUN_STATES : QSO_SP_STATES
    this.qsoStep = (this.qsoStep + 1) % states.length
    const state = states[this.qsoStep]
    let { dxcall, rxReport } = qso

    if (this.qsoStep === 0) {
      // start of a new cycle: no DX yet for a runner, fresh DX for S&P
      dxcall = this.qsoRunning ? null : pickStation(this.snap.stations)
      rxReport = null
    } else if (this.qsoStep === 1) {
      // a station has answered (runner) / we answered a CQ (S&P)
      dxcall = dxcall ?? pickStation(this.snap.stations)
    } else if (this.qsoStep >= 2 && rxReport === null) {
      // report exchanged
      rxReport = -1 * (6 + Math.floor(Math.random() * 16))
    }

    // Plausible "Now sending" text for the demo, tracking the state.
    const me = this.settings.mycall
    const grid = this.settings.mygrid
    let txNow: string | null = qso.txNow ?? null
    if (this.qsoRunning && !dxcall) txNow = `CQ ${me} ${grid}`
    else if (dxcall && rxReport === null) txNow = `${dxcall} ${me} ${grid}`
    else if (dxcall) txNow = `${dxcall} ${me} R${rxReport}`
    return { state, dxcall, rxReport, running: this.qsoRunning, txNow, stalled: false }
  }

  /** Grow the Field Day log + scoreboard a contact at a time (no dupes). */
  private advanceFieldDay(fd: FieldDayStatus): FieldDayStatus {
    // log a new contact most slots (a quick run rate for the demo)
    if (Math.random() < 0.85) {
      const next = randomFieldDayQso()
      // the engine de-dupes: never log a call that is already in the log
      const have = new Set(fd.log.map((q) => q.call))
      if (have.has(next.call)) {
        // pick any pool call not yet worked; if all worked, skip this slot
        const fresh = FD_CALL_POOL.find((c) => !have.has(c))
        if (!fresh) return fd
        next.call = fresh
      }
      const log = [...fd.log, next].slice(-50) // cap demo log length
      const qsoCount = fd.qsoCount + 1
      const sections = uniqueSections(log)
      return {
        ...fd,
        log,
        qsoCount,
        sections,
        points: fdPoints(qsoCount),
      }
    }
    return fd
  }

  private deliverIncoming(
    conversations: Conversation[],
    peer: string,
    slot: number,
  ): Conversation[] {
    const station = this.snap.stations.find((s) => s.call === peer)
    const text = INCOMING_LINES[Math.floor(Math.random() * INCOMING_LINES.length)]
    const incoming = msg({
      from: peer,
      text,
      slot,
      outbound: false,
      directedToMe: true,
      snr: station ? station.snr : -15,
      freqHz: this.snap.link.freqHz + Math.round((Math.random() - 0.5) * 6),
      dtSec: Math.round((Math.random() - 0.5) * 4) / 10,
      tier: this.snap.link.tier,
    })
    const idx = conversations.findIndex((c) => c.peer === peer)
    if (idx === -1) {
      return [...conversations, { peer, messages: [incoming] }]
    }
    const next = conversations.slice()
    next[idx] = { peer, messages: [...next[idx].messages, incoming] }
    return next
  }

  /**
   * Build the next RX slot's decode feed. Occasionally injects a brand-new
   * unseen station, a CQ, or a call directed at me so the live feed + alerts
   * look alive. Keeps the list short (newest first).
   */
  private rollDecodes(prev: DecodeRow[]): DecodeRow[] {
    // ~45% of slots produce a new headline decode
    if (Math.random() > 0.45) return prev
    this.decodeSeq += 1
    const roll = Math.random()
    let row: DecodeRow
    if (roll < 0.25) {
      // a call directed at me
      const s = pickStationObj(this.snap.stations)
      row = decode({
        from: s?.call ?? 'N0GHI',
        message: `${this.snap.mycall} ${s?.call ?? 'N0GHI'} ${-(8 + Math.floor(Math.random() * 14))}`,
        snr: s?.snr ?? -13,
        freqHz: this.snap.link.freqHz + Math.round((Math.random() - 0.5) * 80),
        directedToMe: true,
        worked: s?.worked ?? false,
        tier: this.snap.link.tier,
      })
    } else if (roll < 0.55) {
      // a fresh, never-before-seen station calling CQ
      const fresh = `W${this.decodeSeq % 9}${'ABCDEFGHJ'[this.decodeSeq % 9]}${'XYZQ'[this.decodeSeq % 4]}`
      row = decode({
        from: fresh,
        message: `CQ ${fresh} ${['FN42', 'DM79', 'EL29', 'CN85'][this.decodeSeq % 4]}`,
        snr: -6 - Math.floor(Math.random() * 16),
        freqHz: 300 + Math.round(Math.random() * 2200),
        isCq: true,
        worked: false,
        tier: Math.random() < 0.3 ? 'DX1' : 'FT1',
      })
    } else {
      // an existing roster station calling CQ
      const s = pickStationObj(this.snap.stations)
      row = decode({
        from: s?.call ?? 'K2DEF',
        message: `CQ ${s?.call ?? 'K2DEF'} ${s?.grid ?? 'EN52'}`,
        snr: s?.snr ?? -10,
        freqHz: this.snap.link.freqHz + Math.round((Math.random() - 0.5) * 120),
        isCq: true,
        worked: s?.worked ?? false,
        tier: this.snap.link.tier,
      })
    }
    return [row, ...prev].slice(0, 12)
  }

  /** Inject an open broadcast from a random station into the "*" feed. */
  private deliverBroadcast(conversations: Conversation[], slot: number): Conversation[] {
    const from = BROADCASTERS[Math.floor(Math.random() * BROADCASTERS.length)]
    const station = this.snap.stations.find((s) => s.call === from)
    const template = BROADCAST_LINES[Math.floor(Math.random() * BROADCAST_LINES.length)]
    const text = template
      .replace('{call}', from)
      .replace('{grid}', station?.grid ?? 'EN52')
    const incoming = broadcastMsg({
      from,
      text,
      slot,
      outbound: false,
      snr: station ? station.snr : -15,
      freqHz: this.snap.link.freqHz + Math.round((Math.random() - 0.5) * 80),
      dtSec: Math.round((Math.random() - 0.5) * 4) / 10,
      tier: this.snap.link.tier,
    })
    const idx = conversations.findIndex((c) => c.peer === BROADCAST_PEER)
    if (idx === -1) {
      return [...conversations, { peer: BROADCAST_PEER, messages: [incoming] }]
    }
    const next = conversations.slice()
    next[idx] = {
      peer: BROADCAST_PEER,
      messages: [...next[idx].messages, incoming].slice(-60),
    }
    return next
  }

  private appendMessage(peer: string, m: ChatMessage): void {
    const conversations = this.snap.conversations.slice()
    const idx = conversations.findIndex((c) => c.peer === peer)
    if (idx === -1) {
      conversations.push({ peer, messages: [m] })
    } else {
      conversations[idx] = { peer, messages: [...conversations[idx].messages, m] }
    }
    this.snap = { ...this.snap, conversations }
  }

  private emit(): void {
    for (const fn of this.listeners) fn(this.snap)
  }
}

function pickStation(stations: Station[]): string | null {
  if (stations.length === 0) return null
  const active = stations.filter((s) => s.presence !== 'stale')
  const pool = active.length ? active : stations
  return pool[Math.floor(Math.random() * pool.length)].call
}

function pickStationObj(stations: Station[]): Station | null {
  if (stations.length === 0) return null
  const active = stations.filter((s) => s.presence !== 'stale')
  const pool = active.length ? active : stations
  return pool[Math.floor(Math.random() * pool.length)]
}

function agePresence(s: Station, slot: number): Station {
  const age = slot - s.lastHeardSlot
  let presence = s.presence
  if (age <= 2) presence = 'active'
  else if (age <= 10) presence = 'idle'
  else presence = 'stale'
  return { ...s, presence }
}

function clamp01(v: number): number {
  return Math.max(0, Math.min(1, v))
}
function clampRound(v: number, lo: number, hi: number): number {
  return Math.round(Math.max(lo, Math.min(hi, v)))
}

export const mockEngine = new MockEngine()

// A self-advancing waterfall row generator. Each call returns a new spectrum
// row with a few moving "signals" so the canvas always has fresh data.
let wfPhase = 0
export function nextSpectrumRow(transmitting: boolean): Spectrum {
  wfPhase += 1
  const spec = freshSpectrum()
  const row = spec.row
  // a couple of slowly drifting carriers (decoded signals)
  const carriers = [
    20 + 6 * Math.sin(wfPhase * 0.03),
    52 + 4 * Math.sin(wfPhase * 0.018 + 1),
    78 + 5 * Math.sin(wfPhase * 0.024 + 2),
  ]
  for (const c of carriers) {
    const center = Math.round(c)
    for (let d = -2; d <= 2; d++) {
      const i = center + d
      if (i < 0 || i >= row.length) continue
      const v = (1 - Math.abs(d) / 3) * (0.55 + Math.random() * 0.3)
      row[i] = Math.min(1, row[i] + v)
    }
  }
  if (transmitting) {
    // our own TX trace, bright, near the link freq bin (~1500 Hz -> bin 50)
    for (let d = -2; d <= 2; d++) {
      const i = 50 + d
      if (i < 0 || i >= row.length) continue
      row[i] = Math.min(1, row[i] + (1 - Math.abs(d) / 3))
    }
  }
  return spec
}
