import { useEffect, useRef, useState } from 'react'
import type { AudioDevices, BandChannel, CatTestResult, DetectedRig, RadioStatus, Settings } from '../types'
import {
  clearCloudlogKey,
  clearClublogPassword,
  clearEqslPassword,
  clearHamqthPassword,
  clearHrdlogCode,
  clearLotwPassword,
  clearQrzLogbookKey,
  clearQrzPassword,
  detectRigs,
  downloadEqslReport,
  downloadLotwReport,
  getAllRigModels,
  getAudioDevices,
  getBandPlan,
  getRigModels,
  getSerialPorts,
  getSettings,
  setCloudlogKey,
  setClublogPassword,
  setEqslPassword,
  setHamqthPassword,
  setHrdlogCode,
  setLotwPassword,
  setQrzLogbookKey,
  setQrzPassword,
  setSettings,
  addRadio,
  removeRadio,
  renameRadio,
  setActiveRadio,
  setRadioBands,
  testCat,
  probeCatPorts,
  qrzTestConnection,
  n3fjpTestConnection,
} from '../api'
import { pushToast, withErrorToast } from '../toast'
import { loadProfiles, saveProfile, deleteProfile, type Profile } from '../profiles'
import { getConnectionLog, getCredentialsStatus } from '../api'
import { fetchLotwUsers, getLotwUsersStatus, type LotwUsersStatus } from '../api'
import { discoverFlex } from '../api'
import { civDiagnosticLog, civDiagnosticStatus } from '../api'
import { findDaxDevices, isDaxPaired } from '../features/dax'
import type { ConnEvent, CredStatus } from '../types'
import { FrequencyControl } from './FrequencyControl'
import { LevelMeter } from './LevelMeter'
import { WatchlistPanel } from './WatchlistPanel'
import { MiniSpectrum } from './MiniSpectrum'
import { SettingsGroup } from './SettingsGroup'
import type { Layout } from '../useLayout'
import type { Scale } from '../useScale'
import { SCALE_STEPS } from '../useScale'
import type { FeaturesApi } from '../useFeatures'
import { FEATURES, featureById, type FeatureCategory, type FeatureDef, type FeatureId } from '../features/registry'
import { PROFILE_LIST } from '../features/profiles'
import { checkForUpdateManual } from '../features/updateCheck'
import { ARRL_SECTIONS_BY_DIVISION } from '../features/arrlSections'

interface Props {
  /** Called after a successful save so the shell can refresh its snapshot. */
  onSaved?: () => void
  /** Live radio status, so the Audio section can show the real RX meter. */
  radio?: RadioStatus
  /** The live active radio id (dual-radio). The form reloads when this changes so a switch made from
   * the always-visible TopBar pills while Settings is open can't leave the Rig form stale. */
  activeRadioId?: number
  /** Workspace layout/scale prefs (UI-only — applied live, not via setSettings). */
  layout: Layout
  onLayoutChange: (l: Layout) => void
  scale: Scale
  onScaleChange: (s: Scale) => void
  onResetLayout: () => void
  /** Modular-features API (toggles + profiles). */
  features: FeaturesApi
  /** Re-open the first-run setup wizard. */
  onRerunWizard?: () => void
}

/** Display order for the Features section's category groups. */
const FEATURE_CATEGORY_ORDER: FeatureCategory[] = [
  'Operate',
  'DX & Awards',
  'Propagation',
  'Contesting',
  'POTA/SOTA',
  'Logging',
  'System',
]

type FieldKey = keyof Settings

// Flat ARRL/RAC section list + validity set for the Field Day section picker
// (spec §6). Derived once from the grouped universe; the datalist shows every
// section (code + name + division) and any typed value is validated against
// this set so an operator can only ship a real section in the Cabrillo log.
const FD_SECTION_OPTIONS = ARRL_SECTIONS_BY_DIVISION.flatMap((d) => d.sections)
const FD_SECTION_CODES = new Set(FD_SECTION_OPTIONS.map((s) => s.code))

// TODO (spec §6 grid-seed, deferred): pre-suggest the section from the operator's
// Maidenhead grid for single-section US states. This needs grid → lat/lon → state
// polygon lookup (point-in-polygon against us-atlas) → state → section, which is not
// trivially derivable on the frontend today (no `state_for_grid` mirror exists here);
// the honest single-section states are also a subset that a naive map would get wrong.
// Skipping rather than fabricating — the datalist + validation above already make the
// section fast and correct to pick by hand.

interface FieldDef {
  key: FieldKey
  label: string
  type: 'text' | 'number'
  placeholder: string
  hint?: string
}

// Operator basics (band / dial / sideband are handled by FrequencyControl).
const BASIC_FIELDS: FieldDef[] = [
  { key: 'mycall', label: 'Callsign', type: 'text', placeholder: 'KD9TAW', hint: 'Your station callsign (required).' },
  { key: 'mygrid', label: 'Grid', type: 'text', placeholder: 'EN52', hint: 'Maidenhead locator.' },
  { key: 'opName', label: 'Operator name', type: 'text', placeholder: 'Seth', hint: 'Used by the CW {NAME} macro and logging.' },
]

const PTT_METHODS: { value: string; label: string }[] = [
  { value: 'cat', label: 'CAT (via rigctld)' },
  { value: 'rts', label: 'Serial RTS' },
  { value: 'dtr', label: 'Serial DTR' },
  { value: 'vox', label: 'VOX (no keying)' },
]

// Standard EIA CTCSS (PL) tones, Hz — for the FM repeater-access tone picker.
const CTCSS_TONES = [
  67.0, 71.9, 74.4, 77.0, 79.7, 82.5, 85.4, 88.5, 91.5, 94.8, 97.4, 100.0, 103.5, 107.2,
  110.9, 114.8, 118.8, 123.0, 127.3, 131.8, 136.5, 141.3, 146.2, 151.4, 156.7, 162.2,
  167.9, 173.8, 179.9, 186.2, 192.8, 203.5, 210.7, 218.1, 225.7, 233.6, 241.8, 250.3,
]

const NUMERIC_KEYS: FieldKey[] = ['dialMhz', 'baud', 'rigctldPort', 'rigModel', 'txWatchdogMin', 'catBrokerPort', 'tuneTimeoutSecs']

// Standard serial CAT baud rates offered in the Rig baud picker. A rig's manual lists its
// supported rate(s); most modern rigs run 38400 or 115200. Auto-detect may set a value outside
// this list — the picker keeps it as an extra option so it's never silently dropped.
const STANDARD_BAUDS = [1200, 2400, 4800, 9600, 19200, 38400, 57600, 115200]

/** WSJT-X Split Operation choices (Settings ▸ Radio parity). */
const SPLIT_MODES: { value: NonNullable<Settings['splitMode']>; label: string }[] = [
  { value: 'none', label: 'None' },
  { value: 'rig', label: 'Rig' },
  { value: 'fakeit', label: 'Fake It' },
]

/** One operator override of the working-frequency table. */
type WorkingFrequency = NonNullable<Settings['workingFrequencies']>[number]

/** The stock WSJT-X working-frequency table, shown read-only for reference.
 * An override replaces the matching band+mode row; no overrides = stock. */
const STOCK_WORKING_FREQUENCIES: WorkingFrequency[] = [
  { band: '160m', mode: 'FT8', mhz: 1.84 },
  { band: '80m', mode: 'FT8', mhz: 3.573 },
  { band: '60m', mode: 'FT8', mhz: 5.357 },
  { band: '40m', mode: 'FT8', mhz: 7.074 },
  { band: '30m', mode: 'FT8', mhz: 10.136 },
  { band: '20m', mode: 'FT8', mhz: 14.074 },
  { band: '17m', mode: 'FT8', mhz: 18.1 },
  { band: '15m', mode: 'FT8', mhz: 21.074 },
  { band: '12m', mode: 'FT8', mhz: 24.915 },
  { band: '10m', mode: 'FT8', mhz: 28.074 },
  { band: '6m', mode: 'FT8', mhz: 50.313 },
  { band: '2m', mode: 'FT8', mhz: 144.174 },
  { band: '70cm', mode: 'FT8', mhz: 432.065 },
  { band: '23cm', mode: 'FT8', mhz: 1296.174 },
  { band: '80m', mode: 'FT4', mhz: 3.575 },
  { band: '40m', mode: 'FT4', mhz: 7.0475 },
  { band: '30m', mode: 'FT4', mhz: 10.14 },
  { band: '20m', mode: 'FT4', mhz: 14.08 },
  { band: '17m', mode: 'FT4', mhz: 18.104 },
  { band: '15m', mode: 'FT4', mhz: 21.14 },
  { band: '12m', mode: 'FT4', mhz: 24.919 },
  { band: '10m', mode: 'FT4', mhz: 28.18 },
  { band: '6m', mode: 'FT4', mhz: 50.318 },
  { band: '2m', mode: 'FT4', mhz: 144.17 },
]

/** Bands/modes offered in the override editor (the stock table's coverage). */
const FREQ_BANDS = ['160m', '80m', '60m', '40m', '30m', '20m', '17m', '15m', '12m', '10m', '6m', '2m', '70cm', '23cm']
const FREQ_MODES = ['FT8', 'FT4']

/** Settings is split into tabbed sections: only the active one renders, so a
 * keystroke re-renders ~one section's worth of inputs instead of the whole panel
 * (fixes typing lag) — and it tames the single-giant-scroll wall. */
type SettingsTab =
  | 'station'
  | 'rig'
  | 'audio'
  | 'operating'
  | 'frequencies'
  | 'alerts'
  | 'connections'
  | 'confirmations'
  | 'features'
  | 'workspace'
  | 'fieldday'

const SETTINGS_TABS: { id: SettingsTab; label: string }[] = [
  { id: 'station', label: 'Station' },
  { id: 'rig', label: 'Rig / CAT' },
  { id: 'audio', label: 'Audio' },
  { id: 'operating', label: 'Operating' },
  { id: 'frequencies', label: 'Frequencies' },
  { id: 'alerts', label: 'Alerts' },
  { id: 'connections', label: 'Integrations & Feeds' },
  { id: 'confirmations', label: 'Logbook & QSL' },
  { id: 'features', label: 'Features' },
  { id: 'workspace', label: 'Workspace' },
  { id: 'fieldday', label: 'Field Day' },
]

// Public human DX-cluster nodes (SSB/phone + human spots) — the RBN CW + digital skimmer
// feeds connect automatically, so these are for the phone/human spots RBN doesn't carry.
// Researched, community-trusted, callsign-only login. (NOT RBN ports — those are wired
// separately; the global human-spot mesh means any well-connected node has the same spots.)
const CLUSTER_PRESETS: { label: string; host: string }[] = [
  { label: 'VE7CC-1 — human SSB/CW, clean (recommended)', host: 've7cc.net:23' },
  { label: 'WA9PIE-2 — port 8000 (use if port 23 is blocked)', host: 'dxc.wa9pie.net:8000' },
  { label: 'W1NR — DXSpider, phone-rich', host: 'dx.w1nr.net:23' },
  { label: 'W3LPL — firehose (skimmer-heavy)', host: 'w3lpl.net:7373' },
]

export function SettingsPanel({
  onSaved,
  radio,
  activeRadioId,
  layout,
  onLayoutChange,
  scale,
  onScaleChange,
  onResetLayout,
  features,
  onRerunWizard,
}: Props) {
  const [form, setForm] = useState<Settings | null>(null)
  const [status, setStatus] = useState<'idle' | 'loading' | 'saving' | 'saved'>('loading')
  const [error, setError] = useState<string | null>(null)
  const [rigModels, setRigModels] = useState<[number, string][]>([])
  // Full Hamlib catalog (thousands of entries) — fetched lazily only when the
  // operator checks "Show all models", so the common case (curated ~50) stays fast.
  const [allRigModels, setAllRigModels] = useState<[number, string][]>([])
  const [allRigModelsLoading, setAllRigModelsLoading] = useState(false)
  const [showAllRigModels, setShowAllRigModels] = useState(false)
  const [serialPorts, setSerialPorts] = useState<string[]>([])
  // Native CI-V bus diagnostic log: null = off, string = the log file path while capturing.
  // Transient (not persisted) — a support tool the operator arms to capture a fault. The
  // backend keeps logging while Settings is closed (so the operator can go transmit), so the
  // toggle reads its real state on mount instead of resetting to "off" on every return here.
  const [civLogPath, setCivLogPath] = useState<string | null>(null)
  useEffect(() => {
    let alive = true
    civDiagnosticStatus()
      .then((p) => {
        if (alive) setCivLogPath(p === '' ? null : p)
      })
      .catch(() => {})
    return () => {
      alive = false
    }
  }, [])
  const [profiles, setProfiles] = useState<Profile[]>(() => loadProfiles())
  const [selectedProfile, setSelectedProfile] = useState('')
  const [newProfileName, setNewProfileName] = useState('')
  const [bandPlan, setBandPlan] = useState<BandChannel[]>([])
  const [audio, setAudio] = useState<AudioDevices>({ input: [], output: [] })
  const [portsLoading, setPortsLoading] = useState(false)
  const [audioLoading, setAudioLoading] = useState(false)
  const [detected, setDetected] = useState<DetectedRig[]>([])
  const [detectedFlex, setDetectedFlex] = useState<{ model: string; nickname: string; ip: string }[]>([])
  const [detecting, setDetecting] = useState(false)
  const [catTesting, setCatTesting] = useState(false)
  const [catResult, setCatResult] = useState<CatTestResult | null>(null)
  // Connections visibility: stored-credential status + the rolling event log —
  // the answer to "I hit save and couldn't tell anything happened".
  const [creds, setCreds] = useState<CredStatus[]>([])
  // ARRL LoTW user-activity list (the decode/roster LoTW marks).
  // Rotator "Other model" entry: UI mode + text live in LOCAL state so a
  // sentinel can never leak into the form (review catch: -1 in the payload
  // failed serde's u32 and rejected the ENTIRE settings save).
  // Find-my-Flex discovery (network rig section).
  const [rotOther, setRotOther] = useState(false)
  const [rotCustom, setRotCustom] = useState('')
  const [lotwUsers, setLotwUsers] = useState<LotwUsersStatus | null>(null)
  const [lotwFetching, setLotwFetching] = useState(false)
  useEffect(() => {
    getLotwUsersStatus()
      .then(setLotwUsers)
      .catch(() => {})
  }, [])
  // "Saved" must not linger forever (it read as a stale artifact) — fade it out.
  // QRZ connection test: a real STATUS round-trip (validates the Logbook API
  // key without inserting anything). idle | testing | the result line.
  const [qrzTest, setQrzTest] = useState<{ state: 'idle' | 'testing' | 'ok' | 'fail'; msg: string }>({ state: 'idle', msg: '' })
  const runQrzTest = () => {
    setQrzTest({ state: 'testing', msg: 'testing…' })
    qrzTestConnection()
      .then((msg) => setQrzTest({ state: 'ok', msg }))
      .catch((e) => setQrzTest({ state: 'fail', msg: String(e) }))
  }
  const [n3fjpTest, setN3fjpTest] = useState<{ state: 'idle' | 'testing' | 'ok' | 'fail'; msg: string }>({ state: 'idle', msg: '' })
  const runN3fjpTest = () => {
    setN3fjpTest({ state: 'testing', msg: 'testing…' })
    n3fjpTestConnection()
      .then((msg) => setN3fjpTest({ state: 'ok', msg }))
      .catch((e) => setN3fjpTest({ state: 'fail', msg: String(e) }))
  }
  useEffect(() => {
    if (status !== 'saved') return
    const id = window.setTimeout(() => setStatus('idle'), 2500)
    return () => window.clearTimeout(id)
  }, [status])
  const [connLog, setConnLog] = useState<ConnEvent[]>([])
  useEffect(() => {
    let live = true
    const load = () => {
      getCredentialsStatus().then((c) => live && setCreds(c)).catch(() => {})
      getConnectionLog().then((l) => live && setConnLog(l)).catch(() => {})
    }
    load()
    const id = window.setInterval(load, 5_000)
    return () => {
      live = false
      window.clearInterval(id)
    }
  }, [])
  // LoTW/eQSL passwords are write-only (kept in the OS keychain, never read back),
  // so they live in local state — not in `form`/Settings.
  const [lotwPw, setLotwPw] = useState('')
  const [lotwSyncing, setLotwSyncing] = useState(false)
  const [eqslPw, setEqslPw] = useState('')
  const [eqslSyncing, setEqslSyncing] = useState(false)
  const [qrzPw, setQrzPw] = useState('')
  const [qrzKey, setQrzKey] = useState('')
  const [hamqthPw, setHamqthPw] = useState('')
  const [clublogPw, setClublogPw] = useState('')
  const [hrdlogCode, setHrdlogCodeField] = useState('')
  const [cloudlogKey, setCloudlogKeyField] = useState('')
  const [tab, setTab] = useState<SettingsTab>('station')
  // In-progress MHz text for the override row being edited — committed only when
  // it parses as a positive number, so a half-typed "14." never corrupts the form.
  const [mhzDraft, setMhzDraft] = useState<{ idx: number; text: string } | null>(null)

  // Dual-radio: if the active radio changes underneath us (the operator used the always-visible
  // TopBar switcher pills while Settings is open), the flat Rig/CAT form now describes the WRONG
  // radio. Reload the form from the live settings so a Save can't fold edits into — or command —
  // the wrong rig. Skip the first observation (the mount effect already loads the form).
  const lastActiveRef = useRef<number | undefined>(undefined)
  useEffect(() => {
    if (activeRadioId == null) return
    if (lastActiveRef.current === undefined) {
      lastActiveRef.current = activeRadioId
      return
    }
    if (lastActiveRef.current === activeRadioId) return
    lastActiveRef.current = activeRadioId
    void getSettings()
      .then((s) => {
        setForm(s)
        dirtyRef.current = false
      })
      .catch(() => {})
  }, [activeRadioId])

  useEffect(() => {
    let mounted = true
    setStatus('loading')
    getSettings()
      .then((s) => {
        if (mounted) {
          setForm(s)
          dirtyRef.current = false
          setStatus('idle')
        }
      })
      .catch(() => mounted && setStatus('idle'))
    getRigModels()
      .then((m) => mounted && setRigModels(m))
      .catch(() => {})
    getSerialPorts()
      .then((p) => mounted && setSerialPorts(p))
      .catch(() => {})
    getBandPlan()
      .then((b) => mounted && setBandPlan(b))
      .catch(() => {})
    getAudioDevices()
      .then((d) => mounted && setAudio(d))
      .catch(() => {})
    return () => {
      mounted = false
    }
  }, [])

  const refreshPorts = () => {
    setPortsLoading(true)
    getSerialPorts()
      .then(setSerialPorts)
      .catch(() => {})
      .finally(() => setPortsLoading(false))
  }

  const refreshAudio = () => {
    setAudioLoading(true)
    getAudioDevices()
      .then(setAudio)
      .catch(() => {})
      .finally(() => setAudioLoading(false))
  }

  const updateNum = (key: FieldKey, value: number) => {
    markDirty()
    setForm((prev) => (prev ? { ...prev, [key]: value } : prev))
  }

  // Tracks unsaved flat-form edits, so switching the active radio (which reloads the form) can warn
  // before discarding them. A ref (not state) — read synchronously in the switch handler, no re-render.
  const dirtyRef = useRef(false)
  const markDirty = () => {
    dirtyRef.current = true
    setStatus('idle')
    setError(null)
  }

  const update = (key: FieldKey, raw: string) => {
    markDirty()
    setForm((prev) => {
      if (!prev) return prev
      if (NUMERIC_KEYS.includes(key)) {
        const num = raw === '' ? 0 : Number(raw)
        return { ...prev, [key]: Number.isNaN(num) ? (prev[key] as number) : num }
      }
      return { ...prev, [key]: raw }
    })
  }

  const updateBool = (key: FieldKey, value: boolean) => {
    markDirty()
    setForm((prev) => (prev ? { ...prev, [key]: value } : prev))
  }

  // The DX-cluster node list (SSB/phone aggregator). Functional update so rapid edits
  // (add/remove/edit a row) never race on a stale `form` capture.
  const mutateClusterHosts = (fn: (hosts: string[]) => string[]) => {
    markDirty()
    setForm((prev) => (prev ? { ...prev, clusterHosts: fn(prev.clusterHosts ?? []) } : prev))
  }

  // Optional numeric fields ('' = null = feature off) — `update` coerces '' to 0,
  // which would silently mean "cap at 0" instead of "no cap".
  const updateNullableNum = (key: FieldKey, raw: string, min: number) => {
    markDirty()
    const v = raw === '' ? null : Math.max(min, Number(raw))
    setForm((prev) =>
      prev ? { ...prev, [key]: Number.isNaN(v as number) ? null : v } : prev,
    )
  }

  // Macros are edited as comma-separated text per context; commit on change.
  // CW F-key macro editor. Empty/unset = the cockpit's built-in defaults; "Customize"
  // seeds the editor from them so the operator tweaks rather than starts blank.
  // The Guided copilot recommends the next F-key by ROLE (CQ → answer → report → 73):
  // customization changes the TEXT each key sends, never the key's job — so the
  // recommended-key highlight and the auto call fill (!) keep working for everyone.
  const CW_MACRO_ROLES: Record<string, string> = {
    F1: 'CQ',
    F2: 'Answer a station',
    F3: 'Send report',
    F4: 'Sign off (73)',
    F5: 'My call',
    F6: 'His call',
    F7: 'Ask repeat',
    F8: 'Query',
  }
  const CW_MACRO_DEFAULTS: { key: string; label: string; text: string }[] = [
    { key: 'F1', label: 'CQ', text: 'CQ CQ DE {MYCALL} {MYCALL} K' },
    { key: 'F2', label: 'Call', text: '! DE {MYCALL} {MYCALL} K' },
    { key: 'F3', label: 'Reply', text: '! DE {MYCALL} UR {RST} {RST} NAME {NAME} {NAME} HW? KN' },
    { key: 'F4', label: '73', text: '! DE {MYCALL} TU 73 SK' },
    { key: 'F5', label: 'My Call', text: '{MYCALL}' },
    { key: 'F6', label: 'His Call', text: '! ' },
    { key: 'F7', label: 'AGN', text: 'AGN AGN' },
    { key: 'F8', label: '?', text: '? ' },
  ]
  // --- CW macro profiles (named F-key sets; the active one drives the cockpit) ---
  // The backend migrates the legacy flat `cw` list into a "Default" profile on load, so
  // `cwProfiles` always has at least one entry. The macro grid below edits the ACTIVE
  // profile's macros; an empty macro list means the cockpit's built-in F1–F8 defaults.
  const cwProfiles = form?.macros.cwProfiles ?? []
  const activeCwIdx = form?.macros.activeCwProfile ?? 0
  const activeCwMacros = cwProfiles[activeCwIdx]?.macros ?? []

  const selectCwProfile = (i: number) => {
    markDirty()
    setForm((prev) => (prev ? { ...prev, macros: { ...prev.macros, activeCwProfile: i } } : prev))
  }
  const addCwProfile = () => {
    const name = window.prompt('New CW macro profile name:', `Profile ${cwProfiles.length + 1}`)?.trim()
    if (!name) return
    markDirty()
    setForm((prev) => {
      if (!prev) return prev
      const list = prev.macros.cwProfiles ?? []
      const cur = list[prev.macros.activeCwProfile ?? 0]?.macros ?? []
      // Seed from the current profile's macros (a copy) so a new set starts editable, or
      // the built-in defaults when the current profile is still on the built-ins (empty).
      const seed = cur.length ? cur.map((m) => ({ ...m })) : CW_MACRO_DEFAULTS.map((m) => ({ ...m }))
      const next = [...list, { name, macros: seed }]
      return { ...prev, macros: { ...prev.macros, cwProfiles: next, activeCwProfile: next.length - 1 } }
    })
  }
  const renameCwProfile = () => {
    const cur = cwProfiles[activeCwIdx]
    if (!cur) return
    const name = window.prompt('Rename CW macro profile:', cur.name)?.trim()
    if (!name) return
    markDirty()
    setForm((prev) => {
      if (!prev) return prev
      const idx = prev.macros.activeCwProfile ?? 0
      const list = (prev.macros.cwProfiles ?? []).map((p, i) => (i === idx ? { ...p, name } : p))
      return { ...prev, macros: { ...prev.macros, cwProfiles: list } }
    })
  }
  const deleteCwProfile = () => {
    if (cwProfiles.length <= 1) return // never delete the last profile
    markDirty()
    setForm((prev) => {
      if (!prev) return prev
      const idx = prev.macros.activeCwProfile ?? 0
      const list = (prev.macros.cwProfiles ?? []).filter((_, i) => i !== idx)
      return {
        ...prev,
        macros: { ...prev.macros, cwProfiles: list, activeCwProfile: Math.min(idx, list.length - 1) },
      }
    })
  }
  // Seed / clear the ACTIVE profile's macros (empty list = the built-in defaults).
  const setCwMacros = (macros: { key: string; label: string; text: string }[]) => {
    markDirty()
    setForm((prev) => {
      if (!prev) return prev
      const idx = prev.macros.activeCwProfile ?? 0
      const list = (prev.macros.cwProfiles ?? []).map((p, i) => (i === idx ? { ...p, macros } : p))
      return { ...prev, macros: { ...prev.macros, cwProfiles: list } }
    })
  }
  const updateCwMacro = (i: number, field: 'label' | 'text', value: string) => {
    markDirty()
    setForm((prev) => {
      if (!prev) return prev
      const idx = prev.macros.activeCwProfile ?? 0
      const list = (prev.macros.cwProfiles ?? []).map((p, pi) => {
        if (pi !== idx) return p
        return { ...p, macros: p.macros.map((m, mi) => (mi === i ? { ...m, [field]: value } : m)) }
      })
      return { ...prev, macros: { ...prev.macros, cwProfiles: list } }
    })
  }

  const updateMacros = (ctx: keyof Settings['macros'], raw: string) => {
    markDirty()
    const list = raw
      .split(',')
      .map((x) => x.trim())
      .filter((x) => x.length > 0)
    setForm((prev) =>
      prev ? { ...prev, macros: { ...prev.macros, [ctx]: list } } : prev,
    )
  }

  // Wanted watch list: comma-separated exact calls or trailing-* wildcard prefixes
  // (e.g. "VP8*, 3Y0J") that raise a loud alert even on a worked station.
  /** WSJT-X Split Operation (none | rig | fakeit). */
  const setSplitMode = (m: NonNullable<Settings['splitMode']>) => {
    markDirty()
    setForm((prev) => (prev ? { ...prev, splitMode: m } : prev))
  }

  // --- working-frequency overrides (Frequencies tab) ---
  const updateOverride = (idx: number, patch: Partial<WorkingFrequency>) => {
    markDirty()
    setForm((prev) => {
      if (!prev) return prev
      const list = [...(prev.workingFrequencies ?? [])]
      if (!list[idx]) return prev
      list[idx] = { ...list[idx], ...patch }
      return { ...prev, workingFrequencies: list }
    })
  }

  const addOverride = () => {
    markDirty()
    setForm((prev) =>
      prev
        ? {
            ...prev,
            workingFrequencies: [
              ...(prev.workingFrequencies ?? []),
              { band: '20m', mode: 'FT8', mhz: 14.074 },
            ],
          }
        : prev,
    )
  }

  const removeOverride = (idx: number) => {
    markDirty()
    setMhzDraft(null)
    setForm((prev) =>
      prev
        ? { ...prev, workingFrequencies: (prev.workingFrequencies ?? []).filter((_, i) => i !== idx) }
        : prev,
    )
  }

  const resetOverrides = () => {
    if (
      (form?.workingFrequencies?.length ?? 0) > 0 &&
      !window.confirm('Clear all working-frequency overrides and go back to the stock WSJT-X table?')
    ) {
      return
    }
    markDirty()
    setMhzDraft(null)
    setForm((prev) => (prev ? { ...prev, workingFrequencies: [] } : prev))
  }

  /** Commit a typed MHz only when valid (positive, finite); otherwise keep prior. */
  const commitMhz = (idx: number, raw: string) => {
    const num = Number(raw)
    if (raw.trim() !== '' && Number.isFinite(num) && num > 0) updateOverride(idx, { mhz: num })
  }

  // Resolve a model's friendly name from whichever list(s) are loaded; an unrecognized
  // number (e.g. typed directly) still commits — Hamlib may support it even unnamed here.
  const findRigModelName = (modelNum: number): string =>
    rigModels.find((m) => m[0] === modelNum)?.[1] ?? allRigModels.find((m) => m[0] === modelNum)?.[1] ?? ''

  const selectRig = (modelNum: number) => {
    markDirty()
    setForm((prev) => (prev ? { ...prev, rigModel: modelNum, rigModelName: findRigModelName(modelNum) } : prev))
  }

  // --- Dual-radio roster (P2). These are LIVE verbs: they persist immediately and return a fresh
  // snapshot. We then re-pull the full settings and merge ONLY the radios[]/active/peg fields back
  // into the form, so the roster reflects the change without discarding unsaved flat-form edits.
  const reloadRadios = () => {
    void getSettings().then((s) => {
      setForm((prev) =>
        prev ? { ...prev, radios: s.radios, activeRadio: s.activeRadio, radioPegged: s.radioPegged } : s,
      )
      onSaved?.()
    })
  }
  const handleAddRadio = () => {
    void withErrorToast(() => addRadio(), 'Could not add a radio').then((s) => s && reloadRadios())
  }
  const handleRemoveRadio = (id: number) => {
    // Destructive + immediate + unrecoverable (drops the radio's CAT/audio config, its unique
    // rigctld port, and band coverage) — confirm before removing.
    const r = form?.radios?.find((p) => p.id === id)
    if (!window.confirm(`Remove ${r?.name ?? 'this radio'}? This deletes its CAT/audio config and can't be undone.`)) {
      return
    }
    void withErrorToast(() => removeRadio(id), 'Could not remove the radio').then((s) => s && reloadRadios())
  }
  const handleRenameRadio = (id: number, name: string) => {
    void withErrorToast(() => renameRadio(id, name), 'Could not rename the radio').then((s) => s && reloadRadios())
  }
  const handleToggleRadioBand = (id: number, band: string) => {
    const radio = form?.radios?.find((r) => r.id === id)
    if (!radio) return
    const bands = radio.bands.includes(band)
      ? radio.bands.filter((b) => b !== band)
      : [...radio.bands, band]
    void withErrorToast(() => setRadioBands(id, bands), 'Could not set band coverage').then(
      (s) => s && reloadRadios(),
    )
  }
  // Switching the active radio is a LIVE rig swap (carrier dropped first) that re-points the flat
  // rig form at that radio — so here we FULLY reload the form (unlike the roster edits above).
  const handleConfigureRadio = (id: number) => {
    // Switching re-points the flat form at the new radio (full reload), discarding any unsaved edits
    // to the current one — warn first if there are any.
    if (dirtyRef.current && !window.confirm('Discard unsaved changes and switch to this radio?')) {
      return
    }
    void withErrorToast(() => setActiveRadio(id), 'Could not switch radios').then((s) => {
      if (!s) return
      void getSettings().then((full) => {
        setForm(full)
        dirtyRef.current = false
      })
      onSaved?.()
    })
  }

  // Lazily fetch the full Hamlib catalog only the first time it's requested.
  const onToggleShowAllRigModels = (checked: boolean) => {
    setShowAllRigModels(checked)
    if (checked && allRigModels.length === 0 && !allRigModelsLoading) {
      setAllRigModelsLoading(true)
      getAllRigModels()
        .then(setAllRigModels)
        .catch(() => {})
        .finally(() => setAllRigModelsLoading(false))
    }
  }

  // Zero-config: scan connected USB radios.
  const onDetectRigs = async () => {
    setDetecting(true)
    // ONE detect for every radio kind (operator request: the USB-only scan
    // could never see a Flex): USB enumeration + LAN discovery in parallel;
    // either probe may fail without killing the other's results.
    const [rigs, flexes] = await Promise.all([
      withErrorToast(() => detectRigs(), 'USB radio detection failed'),
      discoverFlex().catch((e) => {
        pushToast(`Flex LAN scan: ${e instanceof Error ? e.message : e}`, 'info', 6000)
        return []
      }),
    ])
    setDetectedFlex(flexes)
    setDetecting(false)
    if (rigs) {
      setDetected(rigs)
      if (rigs.length === 0 && flexes.length === 0)
        pushToast('No radios found — USB: plug in + power on; Flex: must be on this network.', 'info')
    }
  }

  // One-click apply a detected rig: fill model (if identified) + port + paired audio.
  const applyDetectedRig = (r: DetectedRig) => {
    markDirty()
    setForm((prev) =>
      prev
        ? {
            ...prev,
            ...(r.suggestedModel != null
              ? { rigModel: r.suggestedModel, rigModelName: r.suggestedModelName ?? '' }
              : {}),
            serialPort: r.portName,
            ...(r.suggestedAudio ? { audioIn: r.suggestedAudio, audioOut: r.suggestedAudio } : {}),
          }
        : prev,
    )
    pushToast(
      `Applied ${r.suggestedModelName ?? (r.product || 'radio')} on ${r.portName} — review + Save settings`,
      'success',
    )
  }

  // One-click apply a discovered Flex: network conn via SmartSDR CAT's default
  // slice-A TCP port + the FLEX-6xxx dialect model (the WSJT-X-proven path).
  const applyDetectedFlex = (f: { model: string; nickname: string; ip: string }) => {
    markDirty()
    setForm((prev) =>
      prev
        ? {
            ...prev,
            rigConn: 'network',
            rigAddr: '127.0.0.1:5002',
            rigModel: 2036,
            rigModelName: 'FlexRadio FLEX-6xxx (SmartSDR CAT)',
          }
        : prev,
    )
    pushToast(
      `Applied ${f.model}${f.nickname ? ` "${f.nickname}"` : ''} via SmartSDR CAT (slice A, port 5002) — review + Save, then Test CAT. Second slice? Use port 60001.`,
      'success',
    )
  }

  // FrequencyControl edits the in-form band/dial/sideband; it's persisted on Save.
  const setFreq = (dialMhz: number, band: string, mode: string) => {
    markDirty()
    setForm((prev) =>
      prev ? { ...prev, dialMhz, band: band || prev.band, sideband: mode } : prev,
    )
  }

  // Test CAT (WSJT-X-style): save the form first so the radio loop reconfigures
  // (launching rigctld for CAT) from these exact values, then probe the rig and
  // show a green/red result with the read frequency or a specific error.
  const handleTestCat = async () => {
    if (!form) return
    if (!form.mycall.trim()) {
      setError('Callsign is required.')
      return
    }
    setCatTesting(true)
    setCatResult(null)
    setError(null)
    try {
      await setSettings({ ...form, mycall: form.mycall.trim().toUpperCase() })
      onSaved?.()
      const result = await testCat()
      setCatResult(result)
    } catch {
      setCatResult({ ok: false, detail: 'Could not run the CAT test.' })
    } finally {
      setCatTesting(false)
    }
  }

  // Auto-test ports: probe each USB port (read-only) for the one that actually drives
  // the rig, then auto-fill + save the winning port/baud/model so CAT just works — no
  // guessing which COM port among a rig's several is the control port.
  const handleAutoTestPorts = async () => {
    if (!form) return
    setCatTesting(true)
    setCatResult(null)
    setError(null)
    try {
      const r = await probeCatPorts()
      if (r.found) {
        // Apply port + baud (confirmed working). Only trust the MODEL when it wasn't a guess — a
        // seeded common-rig probe can be answered by a same-family sibling (FT-991A on the FTDX10
        // probe), so keep the operator's Rig Model rather than persisting a wrong one.
        const next = {
          ...form,
          serialPort: r.portName,
          baud: r.baud,
          pttMethod: 'cat',
          ...(r.modelSeeded
            ? {}
            : { rigModel: r.model, rigModelName: r.modelName }),
        }
        setForm(next)
        await setSettings(next)
        onSaved?.()
        setCatResult({ ok: true, detail: `✓ ${r.detail}` })
      } else {
        setCatResult({ ok: false, detail: r.detail })
      }
    } catch {
      setCatResult({ ok: false, detail: 'Could not run the port auto-test.' })
    } finally {
      setCatTesting(false)
    }
  }

  // Config profiles: snapshot the current settings under a name, then switch the whole
  // rig/antenna/CAT/band setup in one move (loading applies via the normal Save path).
  const handleSaveProfile = () => {
    if (!form || !newProfileName.trim()) return
    setProfiles(saveProfile(newProfileName, form))
    pushToast(`Profile "${newProfileName.trim()}" saved`, 'success')
    setNewProfileName('')
  }
  const handleLoadProfile = async () => {
    const p = profiles.find((x) => x.name === selectedProfile)
    if (!p) return
    setForm(p.settings)
    await setSettings(p.settings)
    onSaved?.()
    pushToast(`Loaded profile "${p.name}"`, 'success')
  }
  const handleDeleteProfile = () => {
    if (!selectedProfile) return
    setProfiles(deleteProfile(selectedProfile))
    setSelectedProfile('')
  }

  const onSaveLotwPassword = async () => {
    if (!lotwPw) return
    const ok = await withErrorToast(async () => {
      await setLotwPassword(lotwPw)
      return true
    }, 'Could not save the LoTW password')
    if (ok) {
      setLotwPw('')
      pushToast('LoTW password saved to the system keychain', 'success')
    }
  }

  const onForgetLotwPassword = async () => {
    const ok = await withErrorToast(async () => {
      await clearLotwPassword()
      return true
    }, 'Could not clear the LoTW password')
    if (ok) {
      setLotwPw('')
      pushToast('LoTW password cleared from the keychain', 'success')
    }
  }

  const onSyncLotw = async () => {
    if (!form) return
    setLotwSyncing(true)
    // Persist the form first so the download runs against the username the user
    // sees — the backend reads SAVED settings, not the in-form draft (and a
    // username change resets the sync cursor). Mirrors how Test CAT saves first.
    const r = await withErrorToast(async () => {
      await setSettings({ ...form, mycall: form.mycall.trim().toUpperCase() })
      return downloadLotwReport()
    }, 'LoTW sync failed')
    setLotwSyncing(false)
    if (r) {
      const orphans = r.orphans.length ? ` · ${r.orphans.length} unmatched` : ''
      const promoted = r.promoted ? ` · ${r.promoted} upload${r.promoted === 1 ? '' : 's'} now on file` : ''
      pushToast(
        `LoTW: ${r.newlyConfirmed} newly confirmed, ${r.newlyCredited} credited${promoted}${orphans}`,
        r.orphans.length ? 'info' : 'success',
      )
      onSaved?.()
    }
  }

  const onSaveEqslPassword = async () => {
    if (!eqslPw) return
    const ok = await withErrorToast(async () => {
      await setEqslPassword(eqslPw)
      return true
    }, 'Could not save the eQSL password')
    if (ok) {
      setEqslPw('')
      updateBool('eqslUpload', true)
      pushToast('eQSL password saved — auto-upload to eQSL is ON', 'success')
    }
  }

  const onForgetEqslPassword = async () => {
    const ok = await withErrorToast(async () => {
      await clearEqslPassword()
      return true
    }, 'Could not clear the eQSL password')
    if (ok) {
      setEqslPw('')
      updateBool('eqslUpload', false)
      pushToast('eQSL password cleared — auto-upload to eQSL is off', 'success')
    }
  }

  const onSyncEqsl = async () => {
    if (!form) return
    setEqslSyncing(true)
    // Save the form first so the download uses the username the user sees (the
    // backend reads SAVED settings; a username change resets the cursor).
    const r = await withErrorToast(async () => {
      await setSettings({ ...form, mycall: form.mycall.trim().toUpperCase() })
      return downloadEqslReport()
    }, 'eQSL sync failed')
    setEqslSyncing(false)
    if (r) {
      const orphans = r.orphans.length ? ` · ${r.orphans.length} unmatched` : ''
      // eQSL is non-award-grade, so report newlyConfirmedAny (newlyConfirmed is
      // award-only and always 0 for eQSL).
      pushToast(
        `eQSL: ${r.newlyConfirmedAny} newly confirmed (not DXCC/WAS credit)${orphans}`,
        r.orphans.length ? 'info' : 'success',
      )
      onSaved?.()
    }
  }

  const onSaveQrzPassword = async () => {
    if (!qrzPw) return
    const ok = await withErrorToast(async () => {
      await setQrzPassword(qrzPw)
      return true
    }, 'Could not save the QRZ password')
    if (ok) {
      setQrzPw('')
      pushToast('QRZ password saved to the system keychain', 'success')
    }
  }

  const onForgetQrzPassword = async () => {
    const ok = await withErrorToast(async () => {
      await clearQrzPassword()
      return true
    }, 'Could not clear the QRZ password')
    if (ok) {
      setQrzPw('')
      pushToast('QRZ password cleared from the keychain', 'success')
    }
  }

  const onSaveHamqthPassword = async () => {
    if (!hamqthPw) return
    const ok = await withErrorToast(async () => {
      await setHamqthPassword(hamqthPw)
      return true
    }, 'Could not save the HamQTH password')
    if (ok) {
      setHamqthPw('')
      pushToast('HamQTH password saved to the system keychain', 'success')
    }
  }

  const onForgetHamqthPassword = async () => {
    const ok = await withErrorToast(async () => {
      await clearHamqthPassword()
      return true
    }, 'Could not clear the HamQTH password')
    if (ok) {
      setHamqthPw('')
      pushToast('HamQTH password cleared from the keychain', 'success')
    }
  }

  const onSaveQrzLogbookKey = async () => {
    if (!qrzKey) return
    const ok = await withErrorToast(async () => {
      await setQrzLogbookKey(qrzKey)
      return true
    }, 'Could not save the QRZ Logbook key')
    if (ok) {
      setQrzKey('')
      updateBool('qrzLogbookUpload', true)
      pushToast('QRZ Logbook key saved — auto-upload to QRZ is ON', 'success')
    }
  }

  const onForgetQrzLogbookKey = async () => {
    const ok = await withErrorToast(async () => {
      await clearQrzLogbookKey()
      return true
    }, 'Could not clear the QRZ Logbook key')
    if (ok) {
      setQrzKey('')
      updateBool('qrzLogbookUpload', false)
      pushToast('QRZ Logbook key cleared — auto-upload to QRZ is off', 'success')
    }
  }

  const onSaveClublogPassword = async () => {
    if (!clublogPw) return
    const ok = await withErrorToast(async () => {
      await setClublogPassword(clublogPw)
      return true
    }, 'Could not save the ClubLog password')
    if (ok) {
      setClublogPw('')
      updateBool('clublogUpload', true)
      pushToast('ClubLog app-password saved — auto-upload to ClubLog is ON', 'success')
    }
  }

  const onForgetClublogPassword = async () => {
    const ok = await withErrorToast(async () => {
      await clearClublogPassword()
      return true
    }, 'Could not clear the ClubLog password')
    if (ok) {
      setClublogPw('')
      updateBool('clublogUpload', false)
      pushToast('ClubLog password cleared — auto-upload to ClubLog is off', 'success')
    }
  }

  const onSaveHrdlogCode = async () => {
    if (!hrdlogCode) return
    const ok = await withErrorToast(async () => {
      await setHrdlogCode(hrdlogCode)
      return true
    }, 'Could not save the HRDLog.net upload code')
    if (ok) {
      setHrdlogCodeField('')
      updateBool('hrdlogUpload', true)
      pushToast('HRDLog.net code saved — auto-upload to HRDLog.net is ON', 'success')
    }
  }

  const onForgetHrdlogCode = async () => {
    const ok = await withErrorToast(async () => {
      await clearHrdlogCode()
      return true
    }, 'Could not clear the HRDLog.net upload code')
    if (ok) {
      setHrdlogCodeField('')
      updateBool('hrdlogUpload', false)
      pushToast('HRDLog.net code cleared — auto-upload to HRDLog.net is off', 'success')
    }
  }

  const onSaveCloudlogKey = async () => {
    if (!cloudlogKey) return
    const ok = await withErrorToast(async () => {
      await setCloudlogKey(cloudlogKey)
      return true
    }, 'Could not save the Cloudlog API key')
    if (ok) {
      setCloudlogKeyField('')
      pushToast('Cloudlog API key saved to the keychain', 'success')
    }
  }

  const onForgetCloudlogKey = async () => {
    const ok = await withErrorToast(async () => {
      await clearCloudlogKey()
      return true
    }, 'Could not clear the Cloudlog API key')
    if (ok) {
      setCloudlogKeyField('')
      pushToast('Cloudlog API key cleared from the keychain', 'success')
    }
  }

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    if (!form) return
    if (!form.mycall.trim()) {
      setError('Callsign is required.')
      return
    }
    setStatus('saving')
    setError(null)
    try {
      await setSettings({ ...form, mycall: form.mycall.trim().toUpperCase() })
      dirtyRef.current = false
      setStatus('saved')
      onSaved?.()
    } catch (err) {
      setStatus('idle')
      // Surface the backend's actual message (Tauri rejects with the Err string) — e.g. the
      // dual-radio port-collision rejection tells the operator exactly which ports clash.
      const msg = typeof err === 'string' ? err : err instanceof Error ? err.message : ''
      setError(msg || 'Could not save settings.')
    }
  }

  if (!form) {
    return (
      <section className="panel settings-panel">
        <div className="panel-header">
          <h2>Settings</h2>
        </div>
        <p className="empty">Loading settings…</p>
      </section>
    )
  }

  // One feature toggle row (used by the Core group + each category group).
  const featureRow = (f: FeatureDef) => {
    const on = features.enabled[f.id] !== false
    const depOff = f.dependsOn.find((d) => features.enabled[d] === false)
    return (
      <div className="settings-field" key={f.id}>
        <label className="settings-toggle">
          <span className="settings-label">
            {f.label}
            {f.core && <span className="settings-value"> always on</span>}
          </span>
          <button
            type="button"
            role="switch"
            aria-checked={on}
            disabled={f.core}
            className={`toggle${on ? ' on' : ''}`}
            onClick={() => features.toggle(f.id)}
            aria-label={`${on ? 'Disable' : 'Enable'} ${f.label}`}
          >
            <span className="toggle-knob" />
          </button>
        </label>
        <span className="settings-hint">
          {f.oneLine}
          {depOff && !f.core && ` Turning on also enables ${featureById(depOff as FeatureId)?.label ?? depOff}.`}
        </span>
      </div>
    )
  }

  // serial-port options include the current value even if not in the enumerated
  // list (e.g. a port that has since disappeared), so it stays selectable.
  const portOptions = form.serialPort && !serialPorts.includes(form.serialPort)
    ? [form.serialPort, ...serialPorts]
    : serialPorts

  // include the current selection even if it's not in the enumerated list
  const audioInOptions = form.audioIn && !audio.input.includes(form.audioIn)
    ? [form.audioIn, ...audio.input]
    : audio.input
  const audioOutOptions = form.audioOut && !audio.output.includes(form.audioOut)
    ? [form.audioOut, ...audio.output]
    : audio.output
  // Headphone-monitor device picker: same enumerated-output list, keeping the
  // saved selection visible even if it's since disappeared.
  const monitorOutOptions = form.monitorDevice && !audio.output.includes(form.monitorDevice)
    ? [form.monitorDevice, ...audio.output]
    : audio.output
  // Voice-mic device picker: the enumerated INPUT list (it's a microphone), keeping the
  // saved selection visible even if it's since disappeared.
  const voiceMicOptions = form.voiceMicDevice && !audio.input.includes(form.voiceMicDevice)
    ? [form.voiceMicDevice, ...audio.input]
    : audio.input

  // Frequencies tab: last-wins override lookup for the stock table, plus
  // duplicate band+mode keys (flagged in the editor — the last row wins).
  const overrides = form.workingFrequencies ?? []
  const overrideByKey = new Map<string, number>()
  const dupKeys = new Set<string>()
  for (const o of overrides) {
    const k = `${o.band}|${o.mode}`
    if (overrideByKey.has(k)) dupKeys.add(k)
    overrideByKey.set(k, o.mhz)
  }

  // Field Day section validity: a non-empty value that isn't a known ARRL/RAC
  // section is flagged inline so it never silently reaches the Cabrillo log.
  const fdSectionInvalid =
    form.fdSection.trim() !== '' && !FD_SECTION_CODES.has(form.fdSection.trim().toUpperCase())

  return (
    <section className="panel settings-panel">
      <div className="panel-header">
        <h2>Settings</h2>
        <span className="settings-sub">operator, rig &amp; network</span>
        <span className="settings-build" title="This install's build stamp — confirm a fresh install actually took">
          build {__BUILD_ID__}
        </span>
        <button
          type="button"
          className="settings-update-btn"
          onClick={() => void checkForUpdateManual()}
          title="Check SourceForge for a newer Nexus release"
        >
          Check for updates
        </button>
      </div>

      <form className="settings-form" onSubmit={handleSubmit}>
        <div className="settings-tabs" role="tablist" aria-label="Settings sections">
          {/* The FD-detail tab is hidden unless the Field Day master switch (fdActive,
              toggled in Features) is on — off-season Field Day is entirely invisible. */}
          {SETTINGS_TABS.filter((t) => t.id !== 'fieldday' || form.fdActive).map((t) => (
            <button
              key={t.id}
              type="button"
              role="tab"
              aria-selected={tab === t.id}
              className={`settings-tab${tab === t.id ? ' active' : ''}`}
              onClick={() => setTab(t.id)}
            >
              {t.label}
            </button>
          ))}
        </div>
        <div className="settings-scroll">
          {/* ---- Workspace (UI-only prefs, applied live like the theme) ---- */}
          {tab === 'workspace' && (
          <fieldset className="settings-section">
            <legend>Workspace</legend>
            <div className="settings-grid">
              <div className="settings-field">
                <span className="settings-label">Waterfall position</span>
                <div className="theme-switcher" role="group" aria-label="Waterfall position">
                  {(['right', 'top'] as Layout[]).map((id) => (
                    <button
                      key={id}
                      type="button"
                      className={`theme-chip${layout === id ? ' active' : ''}`}
                      aria-pressed={layout === id}
                      onClick={() => onLayoutChange(id)}
                    >
                      {id === 'right' ? 'Right rail' : 'Top strip'}
                    </button>
                  ))}
                </div>
                <span className="settings-hint">
                  Where the waterfall + decode feed sit. Drag the dividers between panes to resize
                  (double-click a divider to reset).
                </span>
              </div>

              <div className="settings-field">
                <span className="settings-label">UI scale</span>
                <div className="theme-switcher" role="group" aria-label="UI scale">
                  {SCALE_STEPS.map((s) => (
                    <button
                      key={s}
                      type="button"
                      className={`theme-chip${scale === s ? ' active' : ''}`}
                      aria-pressed={scale === s}
                      onClick={() => onScaleChange(s)}
                    >
                      {s}%
                    </button>
                  ))}
                </div>
                <span className="settings-hint">Scales the whole interface; the waterfall stays sharp.</span>
              </div>

              <div className="settings-field">
                <span className="settings-label">Pane sizes</span>
                <button type="button" className="settings-refresh" onClick={onResetLayout}>
                  Reset pane sizes
                </button>
                <span className="settings-hint">Restore the default left/right pane widths.</span>
              </div>
            </div>
          </fieldset>
          )}

          {/* ---- Features (modular toggles + goal profiles) ---- */}
          {tab === 'features' && (
          <fieldset className="settings-section">
            <legend>Features</legend>
            <div className="settings-field">
              <span className="settings-label">Profile</span>
              <div className="theme-switcher settings-profiles" role="group" aria-label="Feature profile">
                {PROFILE_LIST.map((p) => (
                  <button
                    key={p.id}
                    type="button"
                    className={`theme-chip${features.profile === p.id ? ' active' : ''}`}
                    aria-pressed={features.profile === p.id}
                    title={p.blurb}
                    onClick={() => {
                      // Switching from a hand-tuned set discards it — confirm first.
                      if (
                        features.profile !== 'custom' ||
                        window.confirm(`Switch to “${p.label}”? This replaces your custom feature set.`)
                      ) {
                        features.applyProfile(p.id)
                      }
                    }}
                  >
                    {p.label}
                  </button>
                ))}
                {features.profile === 'custom' && (
                  <span className="theme-chip active" aria-disabled="true" title="Custom — a blended feature set (manual toggles or multiple goals)">
                    Custom
                  </span>
                )}
              </div>
              <span className="settings-hint">
                {features.profile === 'custom'
                  ? 'Custom — a blended feature set. Pick a single goal above to reset to its defaults.'
                  : 'Pick a goal to set sensible defaults — every feature stays toggleable below. Switching profiles re-applies its set.'}
                {onRerunWizard && (
                  <>
                    {' '}
                    <button type="button" className="settings-linkbtn" onClick={onRerunWizard}>
                      Re-run setup…
                    </button>
                  </>
                )}
              </span>
            </div>

            {/* Core spine first, as a locked group (spec §4.4). */}
            <div className="settings-featgroup">
              <span className="settings-featgroup-title">Core — always on</span>
              <div className="settings-grid">{FEATURES.filter((f) => f.core).map(featureRow)}</div>
            </div>

            {/* Optional features, grouped by category. Field Day is NOT a plain feature
                toggle: its visibility is driven by the persisted master switch
                (settings.fdActive), so the Contesting group hosts that master toggle in
                place of the old standalone fieldDay flag. */}
            {FEATURE_CATEGORY_ORDER.map((cat) => {
              const inCat = FEATURES.filter((f) => f.category === cat && !f.core && f.id !== 'fieldDay')
              const isContesting = cat === 'Contesting'
              if (inCat.length === 0 && !isContesting) return null
              return (
                <div className="settings-featgroup" key={cat}>
                  <span className="settings-featgroup-title">{cat}</span>
                  <div className="settings-grid">
                    {isContesting && (
                      <div className="settings-field">
                        <label className="settings-toggle">
                          <span className="settings-label">Field Day mode</span>
                          <button
                            type="button"
                            role="switch"
                            aria-checked={!!form.fdActive}
                            className={`toggle${form.fdActive ? ' on' : ''}`}
                            onClick={() => {
                              const next = !form.fdActive
                              updateBool('fdActive', next)
                              // Turning on with no Class/Section yet → jump to the FD setup
                              // tab (now visible) so they're filled; the backend won't enter
                              // Field Day until both are set.
                              if (next && (!form.fdClass.trim() || !form.fdSection.trim())) setTab('fieldday')
                            }}
                            aria-label={`${form.fdActive ? 'Disable' : 'Enable'} Field Day mode`}
                          >
                            <span className="toggle-knob" />
                          </button>
                        </label>
                        <span className="settings-hint">
                          Turn on for Field Day weekend — reveals the Field Day workspace, the
                          Class/Section exchange across all modes, and the setup tab. Off the rest of
                          the year (nothing shows). Stays on across restarts until you turn it off;
                          Save settings to apply.
                        </span>
                      </div>
                    )}
                    {inCat.map(featureRow)}
                  </div>
                </div>
              )
            })}
          </fieldset>
          )}

          {/* ---- Operator & radio ---- */}
          {tab === 'station' && (
          <fieldset className="settings-section">
            <legend>Operator &amp; Radio</legend>
            <div className="settings-grid">
              {BASIC_FIELDS.map((f) => {
                const value = form[f.key]
                const invalid = f.key === 'mycall' && !String(value).trim()
                return (
                  <label className="settings-field" key={f.key}>
                    <span className="settings-label">{f.label}</span>
                    <input
                      className={`settings-input${invalid && error ? ' invalid' : ''}`}
                      type={f.type}
                      value={String(value)}
                      placeholder={f.placeholder}
                      onChange={(e) => update(f.key, e.target.value)}
                      aria-invalid={invalid && !!error}
                      autoComplete="off"
                      spellCheck={false}
                    />
                    {f.hint && <span className="settings-hint">{f.hint}</span>}
                  </label>
                )
              })}
              <label className="settings-field">
                <span className="settings-label">License Class</span>
                <select
                  className="settings-input"
                  value={String(form.licenseClass ?? 'open')}
                  onChange={(e) => update('licenseClass', e.target.value)}
                >
                  <option value="technician">Technician (US)</option>
                  <option value="general">General (US)</option>
                  <option value="extra">Amateur Extra (US)</option>
                  <option value="open">Open — no transmit limits</option>
                </select>
                <span className="settings-hint">
                  Sets your transmit privileges + the licensed-segment band dropdown. Open = no
                  limits (outside the US).
                </span>
              </label>
            </div>
            <div className="settings-freq">
              <span className="settings-label">Band &amp; Frequency</span>
              <FrequencyControl
                channels={bandPlan}
                dialMhz={form.dialMhz}
                band={form.band}
                mode={form.sideband}
                variant="full"
                onSet={setFreq}
              />
              <span className="settings-hint">
                Pick a band-plan channel, or type a dial frequency in MHz.
              </span>
            </div>
          </fieldset>
          )}

          {/* ---- Rig control ---- */}
          {tab === 'rig' && (
          <>
          {/* Dual-radio roster (P2). Always shown — the "+ Add radio" button is the discovery
              affordance a single-radio operator sees; the per-radio list + band coverage only
              matter once there's a 2nd radio. */}
          <fieldset className="settings-section">
            <legend>Radios</legend>
            <div className="radios-manager">
              {(form.radios ?? []).map((r) => {
                const isActive = r.id === form.activeRadio
                const multi = (form.radios?.length ?? 1) > 1
                return (
                  <div key={r.id} className={`radio-card${isActive ? ' active' : ''}`}>
                    <div className="radio-card-head">
                      <input
                        className="settings-input radio-name-input"
                        type="text"
                        defaultValue={r.name}
                        placeholder="Radio name"
                        onBlur={(e) => {
                          const v = e.target.value.trim()
                          if (v && v !== r.name) handleRenameRadio(r.id, v)
                        }}
                        autoComplete="off"
                        spellCheck={false}
                      />
                      {isActive ? (
                        <span
                          className="radio-active-badge"
                          title="The Rig / CAT + Audio settings below configure this radio."
                        >
                          Active
                        </span>
                      ) : (
                        <button
                          type="button"
                          className="settings-refresh"
                          onClick={() => handleConfigureRadio(r.id)}
                          title="Make this the active radio so the Rig / CAT form below configures it (drops the carrier and swaps rigs)."
                        >
                          Configure
                        </button>
                      )}
                      {multi && !isActive && (
                        <button
                          type="button"
                          className="settings-refresh danger"
                          onClick={() => handleRemoveRadio(r.id)}
                          title="Remove this radio from the roster"
                        >
                          Remove
                        </button>
                      )}
                    </div>
                    <div className="radio-card-meta">
                      {r.rigModelName && r.rigModelName !== 'None / VOX'
                        ? r.rigModelName
                        : 'No rig model set'}
                      {' · '}
                      {r.rigConn === 'network' ? r.rigAddr || 'no address' : r.serialPort || 'no COM port'}
                      {' · audio '}
                      {r.audioIn || 'default'}
                      {' · rigctld :'}
                      {r.rigctldPort}
                    </div>
                    {multi && (
                      <div className="radio-band-coverage">
                        <span className="settings-hint">
                          Covers bands (for auto band-routing; none = covers all):
                        </span>
                        <div className="band-chip-row">
                          {FREQ_BANDS.map((b) => {
                            const on = r.bands.includes(b)
                            return (
                              <button
                                key={b}
                                type="button"
                                className={`band-chip${on ? ' on' : ''}`}
                                aria-pressed={on}
                                onClick={() => handleToggleRadioBand(r.id, b)}
                              >
                                {b}
                              </button>
                            )
                          })}
                        </div>
                      </div>
                    )}
                  </div>
                )
              })}
            </div>
            <div className="radios-actions">
              <button type="button" className="settings-refresh" onClick={handleAddRadio}>
                + Add radio
              </button>
              <span className="settings-hint">
                {(form.radios?.length ?? 1) > 1
                  ? `The Rig / CAT + Audio settings below configure “${
                      form.radios?.find((r) => r.id === form.activeRadio)?.name ?? 'the active radio'
                    }”. Each radio has its OWN CAT + audio — use “Configure” (or the top-bar pills) to set up the other radio. “+ Add radio” switches to the new radio automatically.`
                  : 'Run two rigs at once — e.g. an HF radio plus a VHF/UHF radio on a different antenna? Add a second radio, then switch between them from the top bar. Newcomers can ignore this.'}
              </span>
            </div>
          </fieldset>
          <fieldset className="settings-section">
            <legend>Profiles</legend>
            <div className="settings-grid">
              <label className="settings-field">
                <span className="settings-label">Saved profiles</span>
                <div className="settings-input-row">
                  <select
                    className="settings-input"
                    value={selectedProfile}
                    onChange={(e) => setSelectedProfile(e.target.value)}
                  >
                    <option value="">— Select a profile —</option>
                    {profiles.map((p) => (
                      <option key={p.name} value={p.name}>
                        {p.name}
                      </option>
                    ))}
                  </select>
                  <button
                    type="button"
                    className="settings-refresh"
                    onClick={handleLoadProfile}
                    disabled={!selectedProfile}
                    title="Apply this profile (saves it as the active settings)"
                  >
                    Load
                  </button>
                  <button
                    type="button"
                    className="settings-refresh"
                    onClick={handleDeleteProfile}
                    disabled={!selectedProfile}
                  >
                    Delete
                  </button>
                </div>
                <span className="settings-hint">
                  Switch a whole rig / antenna / CAT / band setup in one move.
                </span>
              </label>

              <label className="settings-field">
                <span className="settings-label">Save current as</span>
                <div className="settings-input-row">
                  <input
                    className="settings-input"
                    type="text"
                    value={newProfileName}
                    placeholder="e.g. Portable VHF"
                    onChange={(e) => setNewProfileName(e.target.value)}
                    autoComplete="off"
                    spellCheck={false}
                  />
                  <button
                    type="button"
                    className="settings-refresh"
                    onClick={handleSaveProfile}
                    disabled={!newProfileName.trim()}
                  >
                    Save
                  </button>
                </div>
                <span className="settings-hint">Snapshots the current settings under a name.</span>
              </label>
            </div>
          </fieldset>

          <fieldset className="settings-section">
            <legend>Rig Control</legend>
            <div className="settings-grid">
              <label className="settings-field">
                <span className="settings-label">PTT Method</span>
                <select
                  className="settings-input"
                  value={form.pttMethod}
                  onChange={(e) => update('pttMethod', e.target.value)}
                >
                  {PTT_METHODS.map((m) => (
                    <option key={m.value} value={m.value}>
                      {m.label}
                    </option>
                  ))}
                </select>
                <span className="settings-hint">How transmit is keyed.</span>
              </label>

              <div className="settings-field">
                <span className="settings-label">Zero-config setup</span>
                <div className="settings-input-row">
                  <button
                    type="button"
                    className="settings-refresh"
                    onClick={onDetectRigs}
                    disabled={detecting}
                  >
                    {detecting ? 'Scanning…' : 'Detect my radio'}
                  </button>
                </div>
                {(detected.length > 0 || detectedFlex.length > 0) && (
                  <ul className="rig-detect-list">
                    {detectedFlex.map((f, i) => (
                      <li className="rig-detect" key={`flex-${f.ip}-${i}`}>
                        <div className="rig-detect-main">
                          <span className="rig-detect-name">
                            {f.model}
                            {f.nickname ? ` “${f.nickname}”` : ''} — network
                          </span>
                          <span className="rig-detect-meta">
                            {f.ip} · via SmartSDR CAT on this PC (slice A, TCP 5002)
                          </span>
                        </div>
                        <button type="button" className="settings-save" onClick={() => applyDetectedFlex(f)}>
                          Use this
                        </button>
                      </li>
                    ))}
                    {detected.map((r, i) => (
                      <li className="rig-detect" key={`${r.portName}-${i}`}>
                        <div className="rig-detect-main">
                          <span className="rig-detect-name">
                            {r.suggestedModelName ?? (r.product || 'Unknown radio')}
                          </span>
                          <span className="rig-detect-meta">
                            {r.portName} · {r.chip}
                            {r.suggestedAudio ? ` · ${r.suggestedAudio}` : ''}
                          </span>
                          {!r.suggestedModel && (
                            <span className="rig-detect-nomodel">
                              ⚠ Found the port but not the exact model — common on Yaesu, whose USB
                              bridge reports only its chip name. Pick your rig in <em>Rig Model</em>{' '}
                              below, or click <em>Auto-test</em> (it now tries common rigs to find
                              the right port + baud for you).
                            </span>
                          )}
                          {r.driverNote && !r.driverBundled && (
                            <span className="rig-detect-driver">
                              {r.driverNote}
                              {r.driverUrl && (
                                <>
                                  {' '}
                                  <a href={r.driverUrl} target="_blank" rel="noreferrer">
                                    driver ↗
                                  </a>
                                </>
                              )}
                            </span>
                          )}
                        </div>
                        <button type="button" className="settings-save" onClick={() => applyDetectedRig(r)}>
                          Use this
                        </button>
                      </li>
                    ))}
                  </ul>
                )}
                <span className="settings-hint">
                  One scan for everything: USB radios (fills model, port, sound device)
                  AND FlexRadios on the network (fills the SmartSDR CAT config). Review, then Save.
                </span>
              </div>

              <label className="settings-field">
                <span className="settings-label">Rig Model</span>
                <div className="settings-input-row">
                  <select
                    className="settings-input"
                    value={String(form.rigModel)}
                    onChange={(e) => selectRig(Number(e.target.value))}
                  >
                    <option value="0">— None —</option>
                    {(showAllRigModels ? allRigModels : rigModels).map(([num, name]) => (
                      <option key={num} value={String(num)}>
                        {name} ({num})
                      </option>
                    ))}
                  </select>
                  <input
                    className="settings-input"
                    type="number"
                    inputMode="numeric"
                    min="0"
                    placeholder="or enter model #"
                    onChange={(e) => {
                      const raw = e.target.value
                      const n = Number(raw)
                      if (raw.trim() !== '' && Number.isInteger(n) && n >= 0) {
                        markDirty()
                        setForm((prev) =>
                          prev ? { ...prev, rigModel: n, rigModelName: findRigModelName(n) } : prev,
                        )
                      }
                    }}
                    aria-label="Enter a Hamlib rig model number directly"
                  />
                </div>
                <span className="settings-input-row">
                  <input
                    type="checkbox"
                    checked={showAllRigModels}
                    onChange={(e) => onToggleShowAllRigModels(e.target.checked)}
                    aria-label="Show all Hamlib rig models"
                  />
                  <span className="settings-hint">
                    Show all models{allRigModelsLoading ? ' (loading…)' : ''} — the list above
                    defaults to ~50 curated common rigs; check this for the full Hamlib catalog.
                  </span>
                </span>
                <span className="settings-hint">
                  Hamlib rig model. Not listed? Type its model number directly — Hamlib may
                  still support it even without a friendly name here.
                </span>
              </label>

              <label className="settings-field">
                <span className="settings-label">Connection</span>
                <select
                  className="settings-input"
                  value={form.rigConn || 'serial'}
                  onChange={(e) => update('rigConn', e.target.value)}
                >
                  <option value="serial">Serial (USB / COM)</option>
                  <option value="network">Network (FlexRadio / remote)</option>
                </select>
                <span className="settings-hint">
                  Serial for a USB/COM rig (most, incl. Xiegu); Network for a FlexRadio via
                  SmartSDR or a remote rigctld over TCP.
                </span>
              </label>

              {form.rigConn === 'network' && (
                <label className="settings-field">
                  <span className="settings-label">Network Address</span>
                  <input
                    className="settings-input"
                    type="text"
                    value={form.rigAddr}
                    placeholder="127.0.0.1:5002"
                    onChange={(e) => update('rigAddr', e.target.value)}
                    autoComplete="off"
                    spellCheck={false}
                  />
                  {(() => {
                    const dax = findDaxDevices(audio.input, audio.output)
                    // Bootstrapper, not enforcer: once BOTH sides are any DAX
                    // device (auto or hand-picked), stop offering to "fix" them
                    // — re-pairing over a manual endpoint choice was reverting
                    // the operator's working config (real-6400M report).
                    const paired = isDaxPaired(form.audioIn, form.audioOut)
                    return dax && !paired ? (
                      <button
                        type="button"
                        className="settings-test-btn"
                        onClick={() => {
                          update('audioIn', dax.input)
                          update('audioOut', dax.output)
                          pushToast(`DAX paired: ${dax.input} → in, ${dax.output} → out`, 'success', 6000)
                        }}
                        title="SmartSDR's DAX virtual audio devices were detected — one click sets them as Nexus's audio in/out (bit-clean digital audio, no sound card)"
                      >
                        ⚡ Pair DAX audio ({dax.input})
                      </button>
                    ) : null
                  })()}
                  <span className="settings-hint">
                    host:port. For a Flex: the WSJT-X-proven path is the SmartSDR CAT app
                    on THIS PC — its DEFAULT TCP port 5002 is directed at slice A, so
                    127.0.0.1:5002 with the FLEX-6xxx model works out of the box; audio
                    rides DAX. Multi-slice: SmartSDR CAT's per-slice ports are B=60001,
                    C=60002, D=60003 — Nexus drives ONE slice, so enter the port of the
                    slice you run digital on. (Direct-to-radio :4992 needs Hamlib's
                    experimental native model and failed on real hardware.) Other rigs:
                    a remote rigctld's host:port with their normal model.
                  </span>
                </label>
              )}

              {form.rigConn !== 'network' && (
                <>
              <label className="settings-field">
                <span className="settings-label">Serial Port</span>
                <div className="settings-input-row">
                  <select
                    className="settings-input"
                    value={form.serialPort}
                    onChange={(e) => update('serialPort', e.target.value)}
                  >
                    <option value="">— Select port —</option>
                    {portOptions.map((p) => (
                      <option key={p} value={p}>
                        {p}
                      </option>
                    ))}
                  </select>
                  <button
                    type="button"
                    className="settings-refresh"
                    onClick={refreshPorts}
                    disabled={portsLoading}
                    title="Re-scan serial ports"
                  >
                    {portsLoading ? '…' : 'Refresh'}
                  </button>
                  <button
                    type="button"
                    className="settings-refresh"
                    onClick={handleAutoTestPorts}
                    disabled={catTesting}
                    title="Probe each USB port (read-only — never transmits) and auto-select the one that drives your rig"
                  >
                    {catTesting ? '…' : 'Auto-test'}
                  </button>
                </div>
                <span className="settings-hint">
                  COM / tty device for rig control — or Auto-test to find it.
                </span>
              </label>

              <label className="settings-field">
                <span className="settings-label">Baud</span>
                <select
                  className="settings-input"
                  value={String(form.baud)}
                  onChange={(e) => updateNum('baud', Number(e.target.value))}
                >
                  {(STANDARD_BAUDS.includes(form.baud)
                    ? STANDARD_BAUDS
                    : [form.baud, ...STANDARD_BAUDS]
                  ).map((b) => (
                    <option key={b} value={b}>
                      {b.toLocaleString()}
                    </option>
                  ))}
                </select>
                <span className="settings-hint">
                  Serial baud rate — match your rig's CAT setting (most modern rigs: 38,400 or 115,200).
                </span>
              </label>
                </>
              )}

              <div className="settings-field">
                <span className="settings-label">Antenna rotator</span>
                {(() => {
                  const CURATED = [
                    '0', '601', '603', '602', '901', '902', '202', '204', '401',
                    '403', '405', '1001', '1102', '1701', '1',
                  ]
                  const modelStr = String(form.rotatorModel ?? 0)
                  const isOther = rotOther || !CURATED.includes(modelStr)
                  return (
                    <>
                      <select
                        value={isOther ? 'other' : modelStr}
                        onChange={(e) => {
                          const v = e.target.value
                          if (v === 'other') {
                            setRotOther(true)
                            setRotCustom(
                              (form.rotatorModel ?? 0) > 0 ? String(form.rotatorModel) : '',
                            )
                          } else {
                            setRotOther(false)
                            updateNum('rotatorModel', Number(v))
                          }
                        }}
                        aria-label="Rotator model"
                      >
                        <option value="0">None</option>
                        <option value="601">Yaesu GS-232A</option>
                        <option value="603">Yaesu GS-232B</option>
                        <option value="602">GS-232 (generic)</option>
                        <option value="901">SPID Rot2Prog</option>
                        <option value="902">SPID Rot1Prog</option>
                        <option value="202">EasyComm II</option>
                        <option value="204">EasyComm III</option>
                        <option value="401">Hy-Gain Rotor-EZ</option>
                        <option value="403">Hy-Gain DCU</option>
                        <option value="405">Green Heron RT-21</option>
                        <option value="1001">M2 RC2800</option>
                        <option value="1102">EA4TX ARS (az)</option>
                        <option value="1701">Prosistel D (az)</option>
                        <option value="1">Dummy (testing — no hardware)</option>
                        <option value="other">Other Hamlib model #…</option>
                      </select>
                      {isOther && (
                        <input
                          className="settings-input"
                          type="number"
                          min="1"
                          placeholder="Hamlib rotator model number (rotctl -l lists them)"
                          value={rotCustom}
                          onChange={(e) => {
                            setRotCustom(e.target.value)
                            const n = Number(e.target.value)
                            // Only ever commit a REAL model; an incomplete
                            // entry leaves the last valid value in the form.
                            if (Number.isInteger(n) && n > 0) updateNum('rotatorModel', n)
                          }}
                          aria-label="Hamlib rotator model number"
                        />
                      )}
                    </>
                  )
                })()}
                {(form.rotatorModel ?? 0) > 1 && (
                  <div className="settings-inline-pair">
                    <input
                      className="settings-input"
                      type="text"
                      value={form.rotatorPort ?? ''}
                      placeholder="COM7 / /dev/ttyUSB1"
                      onChange={(e) => update('rotatorPort', e.target.value)}
                      autoComplete="off"
                      spellCheck={false}
                      aria-label="Rotator serial port"
                    />
                    <input
                      className="settings-input"
                      type="number"
                      value={form.rotatorBaud ?? 9600}
                      onChange={(e) => {
                        const n = Number(e.target.value)
                        if (!Number.isNaN(n)) updateNum('rotatorBaud', n)
                      }}
                      aria-label="Rotator baud rate"
                      title="Baud rate (GS-232 default 9600)"
                    />
                  </div>
                )}
                <span className="settings-hint">
                  Pick your rotator and its COM port — Nexus runs the control daemon for you
                  (same as the rig). Then use the Rotor pane in Connect, ↗ on Needed rows,
                  or the compass anywhere.
                </span>
                <input
                  className="settings-input"
                  type="text"
                  value={form.rotatorHost}
                  placeholder="Advanced: external rotctld host:port (overrides the above)"
                  onChange={(e) => update('rotatorHost', e.target.value)}
                  autoComplete="off"
                  spellCheck={false}
                  aria-label="External rotctld address (advanced)"
                />
              </div>

              <label className="settings-field">
                <span className="settings-label">WinKeyer port</span>
                <input
                  className="settings-input"
                  type="text"
                  value={form.winkeyerPort}
                  placeholder="COM6 — K1EL WinKeyer serial port"
                  onChange={(e) => update('winkeyerPort', e.target.value)}
                  autoComplete="off"
                  spellCheck={false}
                />
                <span className="settings-hint">
                  For the WinKeyer CW keyer (select it in the CW cockpit). 1200 baud.
                </span>
              </label>

              <div className="settings-field">
                <span className="settings-label">Split operation</span>
                <div className="theme-switcher" role="group" aria-label="Split operation">
                  {SPLIT_MODES.map((m) => (
                    <button
                      key={m.value}
                      type="button"
                      className={`theme-chip${(form.splitMode ?? 'none') === m.value ? ' active' : ''}`}
                      aria-pressed={(form.splitMode ?? 'none') === m.value}
                      onClick={() => setSplitMode(m.value)}
                    >
                      {m.label}
                    </button>
                  ))}
                </div>
                <span className="settings-hint">
                  Keeps your transmitted audio between 1500–2000 Hz by shifting the TX dial in
                  500 Hz steps, so audio harmonics fall outside the transmit filter — cleaner
                  signal. Rig = uses VFO B split. Fake It = retunes the VFO around each over (works
                  on any CAT rig). None = stock WSJT-X default, transmits at the raw audio offset.
                </span>
              </div>
            </div>
            <SettingsGroup title="Phone / FM" defaultOpen={false}>
              <label className="settings-field">
                <span className="settings-label">Phone mode</span>
                <select
                  className="settings-input"
                  value={form.phoneMode}
                  onChange={(e) => update('phoneMode', e.target.value)}
                >
                  <option value="ssb">SSB (USB/LSB by band)</option>
                  <option value="fm">FM (VHF/UHF + repeaters)</option>
                </select>
                <span className="settings-hint">FM drives the rig to FM + the shift/tone below.</span>
              </label>

              {form.phoneMode === 'fm' && (
                <>
                  <label className="settings-field">
                    <span className="settings-label">Repeater shift</span>
                    <select
                      className="settings-input"
                      value={form.rptrShift}
                      onChange={(e) => update('rptrShift', e.target.value)}
                    >
                      <option value="simplex">Simplex (no shift)</option>
                      <option value="plus">Plus (+)</option>
                      <option value="minus">Minus (−)</option>
                    </select>
                    <span className="settings-hint">Offset is the band standard (2 m 600 k, 70 cm 5 M…).</span>
                  </label>

                  <label className="settings-field">
                    <span className="settings-label">CTCSS (PL) tone</span>
                    <select
                      className="settings-input"
                      value={String(form.ctcssToneHz)}
                      onChange={(e) =>
                        setForm((p) => (p ? { ...p, ctcssToneHz: Number(e.target.value) } : p))
                      }
                    >
                      <option value="0">Off</option>
                      {CTCSS_TONES.map((t) => (
                        <option key={t} value={String(t)}>
                          {t.toFixed(1)} Hz
                        </option>
                      ))}
                    </select>
                    <span className="settings-hint">Repeater access tone (PL).</span>
                  </label>
                </>
              )}
            </SettingsGroup>
            <SettingsGroup title="Advanced" defaultOpen={false}>
              <label className="settings-field">
                <span className="settings-label">rigctld TCP Port</span>
                <input
                  className="settings-input"
                  type="number"
                  inputMode="numeric"
                  value={String(form.rigctldPort)}
                  placeholder="4532"
                  onChange={(e) => update('rigctldPort', e.target.value)}
                  autoComplete="off"
                />
                <span className="settings-hint">Port Nexus launches rigctld on.</span>
              </label>

              {form.rigConn !== 'network' &&
                /IC-?\s?(7300|7610|9700|705|905)\b/i.test(form.rigModelName ?? '') && (
                  <label className="settings-field">
                    <span className="settings-label">Native Icom CI-V (early access)</span>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={form.icomNativeCat ?? false}
                      className={`toggle${form.icomNativeCat ? ' on' : ''}`}
                      onClick={() => updateBool('icomNativeCat', !form.icomNativeCat)}
                    >
                      <span className="toggle-knob" />
                    </button>
                    <span className="settings-hint">
                      Nexus drives this Icom's CI-V directly instead of launching rigctld —
                      unlocking the rig's real spectrum scope in the waterfall ("CI-V RF") and
                      instant dial tracking. The scope stream <strong>requires 115200 baud</strong>:
                      on the rig set Menu ▸ SET ▸ Connectors ▸ CI-V ▸ "CI-V USB Baud Rate" =
                      115200 and "CI-V USB Port" = "Unlink from [REMOTE]", then set Baud above to
                      115200 to match — the rig refuses to stream at lower rates (CAT still works;
                      the scope stays off). Save to apply; turn off any time to return to the
                      classic Hamlib path.
                    </span>
                  </label>
                )}

              {form.rigConn !== 'network' &&
                /IC-?\s?(7300|7610|9700|705|905)\b/i.test(form.rigModelName ?? '') &&
                form.icomNativeCat && (
                  <label className="settings-field">
                    <span className="settings-label">CI-V bus diagnostic log</span>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={civLogPath !== null}
                      className={`toggle${civLogPath !== null ? ' on' : ''}`}
                      onClick={() =>
                        void withErrorToast(
                          () => civDiagnosticLog(civLogPath === null),
                          'Could not toggle the CI-V diagnostic log',
                        ).then((path) => {
                          if (path === undefined) return // error already toasted
                          setCivLogPath(path === '' ? null : path)
                        })
                      }
                    >
                      <span className="toggle-knob" />
                    </button>
                    <span className="settings-hint">
                      {civLogPath !== null ? (
                        <>
                          <strong>Recording</strong> to <code>{civLogPath}</code> — this keeps
                          running while you're on other screens, so go to the FT8 or Phone cockpit
                          and reproduce the issue (Tune or transmit) now. Come back and turn it off
                          when done, then send that file. It shows exactly what's on the bus during
                          the fault.
                        </>
                      ) : (
                        <>
                          Records every byte to/from the radio on the native CI-V path to a file in
                          your Downloads — a support tool for hardware-only issues like the IC-9700
                          PTT flicker. Turn on, reproduce the problem, turn off, then send the file.
                        </>
                      )}
                    </span>
                  </label>
                )}

              {(form.rigModel === 2036 || form.rigModel === 23005) && (
                <label className="settings-field">
                  <span className="settings-label">Flex radio IP (native panadapter)</span>
                  <input
                    className="settings-input"
                    type="text"
                    value={form.flexRadioIp}
                    placeholder="e.g. 192.168.1.50"
                    onChange={(e) => update('flexRadioIp', e.target.value)}
                    autoComplete="off"
                  />
                  <span className="settings-hint">
                    Your FlexRadio's LAN IP (SmartSDR API, port 4992) — turns on the native RF
                    panadapter. This is the <em>radio's</em> address, not the SmartSDR-CAT port above.
                  </span>
                </label>
              )}

              <div className="settings-field">
                <label className="settings-toggle">
                  <span className="settings-label">Share my radio (CAT broker)</span>
                  <button
                    type="button"
                    role="switch"
                    aria-checked={form.catBroker}
                    className={`toggle${form.catBroker ? ' on' : ''}`}
                    onClick={() => updateBool('catBroker', !form.catBroker)}
                  >
                    <span className="toggle-knob" />
                  </button>
                </label>
                <span className="settings-hint">
                  Run a rigctld-compatible server so WSJT-X / N1MM / loggers share this radio THROUGH Nexus
                  (point them at Hamlib NET rigctl, localhost:{form.catBrokerPort}). Restart to apply.
                </span>
              </div>

              {form.catBroker && (
                <label className="settings-field">
                  <span className="settings-label">CAT broker port</span>
                  <input
                    className="settings-input"
                    type="number"
                    inputMode="numeric"
                    value={String(form.catBrokerPort)}
                    placeholder="4532"
                    onChange={(e) => update('catBrokerPort', e.target.value)}
                    autoComplete="off"
                  />
                  <span className="settings-hint">Other apps connect here (Hamlib NET rigctl default 4532).</span>
                </label>
              )}
              {form.catBroker && (
                <div className="settings-field">
                  <span className="settings-label">Broker PTT</span>
                  <button
                    type="button"
                    role="switch"
                    aria-checked={form.catBrokerPtt ?? false}
                    className={`toggle${form.catBrokerPtt ? ' on' : ''}`}
                    onClick={() => updateBool('catBrokerPtt', !form.catBrokerPtt)}
                  >
                    <span className="knob" />
                  </button>
                  <span className="settings-hint">
                    Let the connected app key transmit when Nexus is idle. Off = other apps
                    control the rig but never key it (Nexus owns TX).
                  </span>
                </div>
              )}
            </SettingsGroup>
            <div className="settings-cat-test">
              <button
                type="button"
                className="settings-testcat"
                onClick={handleTestCat}
                disabled={catTesting}
                title="Save settings, connect to the rig, and read its frequency"
              >
                {catTesting ? 'Testing…' : 'Test CAT'}
              </button>
              {(() => {
                // Show the just-run test result, else the live CAT status from the snapshot.
                const ok = catResult ? catResult.ok : radio?.catOk
                const detail = catResult ? catResult.detail : radio?.catDetail
                if (detail == null || detail === '') return null
                const cls = ok === true ? 'ok' : ok === false ? 'fail' : 'na'
                const mark = ok === true ? '✓ ' : ok === false ? '✗ ' : ''
                return (
                  <span className={`cat-result ${cls}`} role="status">
                    {mark}
                    {detail}
                  </span>
                )
              })()}
            </div>
            <p className="settings-note">
              Saving applies your rig settings live (no restart). <strong>Test CAT</strong> saves,
              launches the bundled <code>rigctld</code> (Hamlib ships with Nexus on Windows — no
              separate install), and reads your rig&apos;s frequency to confirm CAT. For CAT, pick
              your <em>Rig Model</em> and <em>Serial Port</em>; serial RTS/DTR and VOX need no model.
            </p>
          </fieldset>
          </>
          )}

          {/* ---- Audio ---- */}
          {tab === 'audio' && (
          <>
          {(form.radios?.length ?? 1) > 1 && (
            <div className="radio-config-banner">
              🎚 Audio devices below are for{' '}
              <strong>
                {form.radios?.find((r) => r.id === form.activeRadio)?.name ?? 'the active radio'}
              </strong>
              . Each radio has its OWN input/output — switch radios (the top-bar pills, or “Configure”
              in Rig ▸ Radios) to set the other radio's audio. The RX audio + waterfall follow whichever
              radio is active.
            </div>
          )}
          <fieldset className="settings-section">
            <legend>Audio</legend>
            <div className="settings-grid">
              <label className="settings-field">
                <span className="settings-label">Input Device (RX)</span>
                <div className="settings-input-row">
                  <select
                    className="settings-input"
                    value={form.audioIn}
                    onChange={(e) => update('audioIn', e.target.value)}
                  >
                    <option value="">System default</option>
                    {audioInOptions.map((d) => (
                      <option key={d} value={d}>
                        {d}
                      </option>
                    ))}
                  </select>
                  <button
                    type="button"
                    className="settings-refresh"
                    onClick={refreshAudio}
                    disabled={audioLoading}
                    title="Re-scan audio devices"
                  >
                    {audioLoading ? '…' : 'Refresh'}
                  </button>
                </div>
                <span className="settings-hint">Sound card carrying receive audio.</span>
              </label>

              <label className="settings-field">
                <span className="settings-label">Output Device (TX)</span>
                <select
                  className="settings-input"
                  value={form.audioOut}
                  onChange={(e) => update('audioOut', e.target.value)}
                >
                  <option value="">System default</option>
                  {audioOutOptions.map((d) => (
                    <option key={d} value={d}>
                      {d}
                    </option>
                  ))}
                </select>
                <span className="settings-hint">Sound card feeding the rig (transmit).</span>
              </label>

              <label className="settings-field">
                <span className="settings-label">Voice mic (recording)</span>
                <select
                  className="settings-input"
                  value={form.voiceMicDevice ?? ''}
                  onChange={(e) => update('voiceMicDevice', e.target.value)}
                >
                  <option value="">Same as audio input (default)</option>
                  {voiceMicOptions.map((d) => (
                    <option key={d} value={d}>
                      {d}
                    </option>
                  ))}
                </select>
                <span className="settings-hint">
                  Mic used when RECORDING a voice-keyer message. Default records from the
                  input device above — but on a digital setup that's the rig's RX audio, so
                  you'd record the band, not your voice. Pick your actual mic here. If it
                  can't open, recording falls back to the input device (never silent).
                </span>
              </label>

              <div className="settings-field settings-audio-scope">
                <span className="settings-label">Live input spectrum</span>
                <MiniSpectrum
                  height={84}
                  idleHint="Flat — no audio on the selected input. Check the device above (radio on? right codec?)."
                />
                <span className="settings-hint">
                  What the selected input hears, live — band noise should show as a moving
                  floor. Confirms the RIGHT device before you leave Settings.
                </span>
              </div>

              <label className="settings-field">
                <span className="settings-label">
                  Tx Power <span className="settings-value">{Math.round(form.txLevel * 100)}%</span>
                </span>
                <input
                  className="settings-slider"
                  type="range"
                  min="0"
                  max="1"
                  step="0.01"
                  value={String(form.txLevel)}
                  onChange={(e) => updateNum('txLevel', Number(e.target.value))}
                  aria-label="Transmit drive level"
                />
                <span className="settings-hint">Transmit drive into the rig (avoid ALC overdrive).</span>
              </label>

              <div className="settings-field">
                <span className="settings-label">RX Level</span>
                <LevelMeter value={radio ? radio.rxLevel : 0} label="RX audio level" variant="full" />
                <span className="settings-hint">Aim for the green zone; red = clipping.</span>
                {radio?.audioError && (
                  <span className="cat-result fail" role="alert">✗ {radio.audioError}</span>
                )}
              </div>
            </div>
          </fieldset>

          <fieldset className="settings-section">
            <legend>Headphone monitor</legend>
            <div className="settings-grid">
              <label className="settings-field">
                <span className="settings-label">Enable monitor</span>
                <span className="settings-input-row">
                  <input
                    type="checkbox"
                    checked={!!form.monitorEnabled}
                    onChange={(e) => updateBool('monitorEnabled', e.target.checked)}
                    aria-label="Enable headphone monitor"
                  />
                  <span className="settings-hint">
                    Plays the exact audio the decoder hears — for level / RFI diagnosis and
                    listening to the band. Off by default; UNVERIFIED on-air until the attended
                    session. Guards against the rig's TX device by name (System default is
                    resolved to its real device first) — if your devices go by multiple
                    names, pick your headphones explicitly rather than System default.
                  </span>
                </span>
              </label>

              <label className="settings-field">
                <span className="settings-label">Monitor Output Device</span>
                <select
                  className="settings-input"
                  value={form.monitorDevice ?? ''}
                  onChange={(e) => update('monitorDevice', e.target.value)}
                  disabled={!form.monitorEnabled}
                >
                  <option value="">System default</option>
                  {monitorOutOptions.map((d) => (
                    <option key={d} value={d}>
                      {d}
                    </option>
                  ))}
                </select>
                <span className="settings-hint">
                  Your headphones or speakers — must NOT be the rig's TX output device.
                </span>
              </label>

              <label className="settings-field">
                <span className="settings-label">
                  Monitor Level{' '}
                  <span className="settings-value">{Math.round((form.monitorLevel ?? 0.5) * 100)}%</span>
                </span>
                <input
                  className="settings-slider"
                  type="range"
                  min="0"
                  max="1"
                  step="0.01"
                  value={String(form.monitorLevel ?? 0.5)}
                  onChange={(e) => updateNum('monitorLevel', Number(e.target.value))}
                  disabled={!form.monitorEnabled}
                  aria-label="Headphone monitor level"
                />
                <span className="settings-hint">Headphone listening volume (does not affect TX).</span>
              </label>
            </div>
          </fieldset>
          </>
          )}

          {/* ---- Operating ---- */}
          {tab === 'operating' && (
          <fieldset className="settings-section">
            <legend>Operating</legend>
            <div className="settings-featgroup">
              <span className="settings-featgroup-title">Transmit &amp; Sequencing</span>
              <div className="settings-grid">
                <div className="settings-field">
                  <label className="settings-toggle">
                    <span className="settings-label">Transmit period — Tx 1st (even)</span>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={form.txEven}
                      className={`toggle${form.txEven ? ' on' : ''}`}
                      onClick={() => updateBool('txEven', !form.txEven)}
                    >
                      <span className="toggle-knob" />
                    </button>
                  </label>
                  <span className="settings-hint">
                    On = transmit in the even/1st T/R slots; off = odd/2nd. The two stations in a QSO
                    must pick <strong>opposite</strong> periods. Also on the top bar (Tx 1st / Tx 2nd).
                  </span>
                </div>

                <label className="settings-field">
                  <span className="settings-label">Tx Watchdog (min)</span>
                  <input
                    className="settings-input"
                    type="number"
                    inputMode="numeric"
                    min="0"
                    value={String(form.txWatchdogMin)}
                    placeholder="6"
                    onChange={(e) => update('txWatchdogMin', e.target.value)}
                    autoComplete="off"
                  />
                  <span className="settings-hint">Auto-halt TX after this many minutes (0 = off).</span>
                </label>

                <div className="settings-field">
                  <label className="settings-toggle">
                    <span className="settings-label">Disable TX after sending 73</span>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={form.disableTxAfter73 !== false}
                      className={`toggle${form.disableTxAfter73 !== false ? ' on' : ''}`}
                      onClick={() => updateBool('disableTxAfter73', form.disableTxAfter73 === false)}
                    >
                      <span className="toggle-knob" />
                    </button>
                  </label>
                  <span className="settings-hint">
                    After your final 73 goes out, Enable TX drops — working the next station is a
                    deliberate arm (WSJT-X default). A CQ run is unaffected: it returns to CQ.
                  </span>
                </div>

                <div className="settings-field">
                  <label className="settings-toggle">
                    <span className="settings-label">Band-edge tones</span>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={form.bandEdgeTones !== false}
                      className={`toggle${form.bandEdgeTones !== false ? ' on' : ''}`}
                      onClick={() => updateBool('bandEdgeTones', form.bandEdgeTones === false)}
                    >
                      <span className="toggle-knob" />
                    </button>
                  </label>
                  <span className="settings-hint">
                    A short audio cue when the dial crosses your license privileges — a rising
                    "ding" back in band, a falling "dong" past an edge.
                  </span>
                </div>

                <div className="settings-field">
                  <label className="settings-toggle">
                    <span className="settings-label">Double-click arms TX</span>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={form.doubleClickSetsTx !== false}
                      className={`toggle${form.doubleClickSetsTx !== false ? ' on' : ''}`}
                      onClick={() => updateBool('doubleClickSetsTx', form.doubleClickSetsTx === false)}
                    >
                      <span className="toggle-knob" />
                    </button>
                  </label>
                  <span className="settings-hint">
                    Double-clicking a station enables TX so the answer goes straight out (WSJT-X
                    "double-click on call sets Tx enable"). Off = you arm TX yourself each time.
                  </span>
                </div>

                <div className="settings-field">
                  <label>
                    <span className="settings-label">Tune timeout (s)</span>
                    <input
                      className="settings-input"
                      type="number"
                      min={1}
                      max={120}
                      value={form.tuneTimeoutSecs || 12}
                      onChange={(e) => {
                        // '' must mean "back to the 12 s default" — the generic
                        // numeric coercion turned a cleared field into a saved 0.
                        markDirty()
                        const n = e.target.value === '' ? 12 : Math.max(1, Number(e.target.value) || 12)
                        setForm((prev) => (prev ? { ...prev, tuneTimeoutSecs: n } : prev))
                      }}
                    />
                  </label>
                  <span className="settings-hint">
                    Auto-release the tune carrier after this many seconds — never leave a key-down
                    unattended.
                  </span>
                </div>
              </div>
            </div>

            <div className="settings-featgroup">
              <span className="settings-featgroup-title">Auto-CQ &amp; Caller Selection</span>
              <div className="settings-grid">
                <div className="settings-field">
                  <label>
                    <span className="settings-label">Stop CQ after N calls</span>
                    <input
                      className="settings-input"
                      type="number"
                      min={1}
                      max={99}
                      value={form.cqMaxCalls ?? ''}
                      placeholder="keep calling"
                      onChange={(e) => updateNullableNum('cqMaxCalls', e.target.value, 1)}
                    />
                  </label>
                  <span className="settings-hint">
                    Blank = WSJT-X behavior: CQ repeats until you stop it (the TX watchdog is the
                    backstop). Set a number to auto-stop an unanswered CQ run after that many calls.
                  </span>
                </div>

                <div className="settings-field">
                  <label>
                    <span className="settings-label">Auto-CQ: drop a silent caller after N overs</span>
                    <input
                      className="settings-input"
                      type="number"
                      min={0}
                      max={99}
                      value={form.cqStallOvers ?? ''}
                      placeholder="3"
                      onChange={(e) => updateNullableNum('cqStallOvers', e.target.value, 0)}
                    />
                  </label>
                  <span className="settings-hint">
                    During an Auto-CQ run, if a station answers then goes silent, abandon it and
                    return to calling CQ after this many unanswered overs. Blank = 3; 0 = never
                    abandon (wait for you, like stock WSJT-X).
                  </span>
                </div>

                <div className="settings-field">
                  <span className="settings-label">Best caller (auto-CQ pick)</span>
                  <div className="settings-input-row">
                    <select
                      className="settings-input"
                      value={form.bestCaller || 'first'}
                      onChange={(e) => update('bestCaller', e.target.value)}
                    >
                      <option value="first">First to answer (default)</option>
                      <option value="strongest">Strongest signal</option>
                      <option value="farthest">Farthest away</option>
                      <option value="cq_first">Prefer CQ callers</option>
                    </select>
                    <input
                      className="settings-input"
                      type="number"
                      inputMode="numeric"
                      value={form.bestCallerMinSnr ?? ''}
                      placeholder="min SNR dB (optional)"
                      onChange={(e) => updateNullableNum('bestCallerMinSnr', e.target.value, -30)}
                      aria-label="Minimum SNR (dB) to consider when picking the best caller"
                    />
                  </div>
                  <span className="settings-hint">
                    When several stations answer your CQ, which to work first.
                  </span>
                </div>
              </div>
            </div>

            <div className="settings-featgroup">
              <span className="settings-featgroup-title">Logging Behavior</span>
              <div className="settings-grid">
                <div className="settings-field">
                  <label className="settings-toggle">
                    <span className="settings-label">Auto-log QSOs</span>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={form.autoLog}
                      className={`toggle${form.autoLog ? ' on' : ''}`}
                      onClick={() => updateBool('autoLog', !form.autoLog)}
                    >
                      <span className="toggle-knob" />
                    </button>
                  </label>
                  <span className="settings-hint">Automatically log completed contacts to the ADIF logbook.</span>
                </div>

                <div className="settings-field">
                  <label className="settings-toggle">
                    <span className="settings-label">Prompt before logging</span>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={!!form.promptToLog}
                      className={`toggle${form.promptToLog ? ' on' : ''}`}
                      onClick={() => updateBool('promptToLog', !form.promptToLog)}
                    >
                      <span className="toggle-knob" />
                    </button>
                  </label>
                  <span className="settings-hint">
                    Show a confirm-and-edit popup when a QSO completes instead of logging silently
                    (WSJT-X “Prompt me to log QSO”). No effect unless Auto-log is on.
                  </span>
                </div>

                <div className="settings-field">
                  <label className="settings-toggle">
                    <span className="settings-label">Roger with RRR (not RR73)</span>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={!!form.preferRrr}
                      className={`toggle${form.preferRrr ? ' on' : ''}`}
                      onClick={() => updateBool('preferRrr', !form.preferRrr)}
                    >
                      <span className="toggle-knob" />
                    </button>
                  </label>
                  <span className="settings-hint">
                    Acknowledge the final report with a bare RRR (partner still owes a 73) instead of
                    the combined RR73. Off = RR73 (modern FT8 practice).
                  </span>
                </div>

                <div className="settings-field">
                  <label className="settings-toggle">
                    <span className="settings-label">Clear DX call after logging</span>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={!!form.clearDxAfterLog}
                      className={`toggle${form.clearDxAfterLog ? ' on' : ''}`}
                      onClick={() => updateBool('clearDxAfterLog', !form.clearDxAfterLog)}
                    >
                      <span className="toggle-knob" />
                    </button>
                  </label>
                  <span className="settings-hint">
                    Wipe the DX Call / DX Grid fields once a contact is logged (WSJT-X option,
                    off by default).
                  </span>
                </div>
              </div>
            </div>

            <div className="settings-featgroup">
              <span className="settings-featgroup-title">Decoder</span>
              <div className="settings-grid">
                <div className="settings-field">
                  <span className="settings-label">Decode depth</span>
                  <div className="theme-switcher" role="group" aria-label="Decode depth">
                    {([1, 2, 3] as const).map((d) => (
                      <button
                        key={d}
                        type="button"
                        className={`theme-chip${(form.decodeDepth ?? 3) === d ? ' active' : ''}`}
                        aria-pressed={(form.decodeDepth ?? 3) === d}
                        onClick={() => {
                          markDirty()
                          setForm((prev) => (prev ? { ...prev, decodeDepth: d } : prev))
                        }}
                      >
                        {d === 1 ? 'Fast' : d === 2 ? 'Normal' : 'Deep'}
                      </button>
                    ))}
                  </div>
                  <span className="settings-hint">
                    Deep finds the most signals (WSJT-X default); Fast saves CPU on old hardware.
                  </span>
                </div>

                <div className="settings-field">
                  <span className="settings-label">Decoder passband (Hz)</span>
                  <div className="settings-input-row">
                    <label className="settings-inline-label">
                      <span>F low</span>
                      <input
                        id="decode-flow"
                        className="settings-input"
                        type="number"
                        inputMode="numeric"
                        min={200}
                        max={2900}
                        step={1}
                        value={form.decodeFLowHz ?? 200}
                        aria-label="Decoder F low (Hz)"
                        onChange={(e) => {
                          if (e.target.value === '') return // mid-edit clear: keep the prior value
                          markDirty()
                          const raw = Number(e.target.value)
                          const clamped = Math.max(200, Math.min(2900, Math.round(raw)))
                          setForm((prev) =>
                            prev
                              ? { ...prev, decodeFLowHz: clamped }
                              : prev,
                          )
                        }}
                        onBlur={() => {
                          setForm((prev) => {
                            if (!prev) return prev
                            const lo = prev.decodeFLowHz ?? 200
                            const hi = prev.decodeFHighHz ?? 2900
                            if (lo >= hi) return { ...prev, decodeFLowHz: Math.min(lo, hi - 1) }
                            return prev
                          })
                        }}
                      />
                    </label>
                    <label className="settings-inline-label">
                      <span>F high</span>
                      <input
                        id="decode-fhigh"
                        className="settings-input"
                        type="number"
                        inputMode="numeric"
                        min={200}
                        max={2900}
                        step={1}
                        value={form.decodeFHighHz ?? 2900}
                        aria-label="Decoder F high (Hz)"
                        onChange={(e) => {
                          if (e.target.value === '') return // mid-edit clear: keep the prior value
                          markDirty()
                          const raw = Number(e.target.value)
                          const clamped = Math.max(200, Math.min(2900, Math.round(raw)))
                          setForm((prev) =>
                            prev
                              ? { ...prev, decodeFHighHz: clamped }
                              : prev,
                          )
                        }}
                        onBlur={() => {
                          setForm((prev) => {
                            if (!prev) return prev
                            const lo = prev.decodeFLowHz ?? 200
                            const hi = prev.decodeFHighHz ?? 2900
                            if (hi <= lo) return { ...prev, decodeFHighHz: Math.max(hi, lo + 1) }
                            return prev
                          })
                        }}
                      />
                    </label>
                  </div>
                  <span className="settings-hint">
                    Restrict the decoder&apos;s search range — useful with narrow filters or strong
                    close-in QRM. Default 200–2900 Hz (full passband).
                  </span>
                </div>

                <div className="settings-field">
                  <span className="settings-label">DXpedition mode</span>
                  <div className="theme-switcher" role="group" aria-label="DXpedition mode">
                    {([
                      { value: 'none' as const, label: 'Off' },
                      { value: 'hound' as const, label: 'Hound' },
                    ]).map((op) => (
                      <button
                        key={op.value}
                        type="button"
                        className={`theme-chip${(form.specialOp ?? 'none') === op.value ? ' active' : ''}`}
                        aria-pressed={(form.specialOp ?? 'none') === op.value}
                        onClick={() => {
                          markDirty()
                          setForm((prev) => prev ? { ...prev, specialOp: op.value } : prev)
                        }}
                      >
                        {op.label}
                      </button>
                    ))}
                  </div>
                  <span className="settings-hint">
                    Off = normal FT8/FT4 operation. Hound = DXpedition pile-up discipline (calls
                    above 1000 Hz; your report auto-moves to the Fox&apos;s frequency).
                  </span>
                </div>
              </div>
            </div>

            <div className="settings-featgroup">
              <span className="settings-featgroup-title">Station Housekeeping</span>
              <div className="settings-grid">
                <div className="settings-field">
                  <label className="settings-toggle">
                    <span className="settings-label">Journey — track a weekly streak</span>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={!!form.journeyStreakEnabled}
                      className={`toggle${form.journeyStreakEnabled ? ' on' : ''}`}
                      onClick={() => updateBool('journeyStreakEnabled', !form.journeyStreakEnabled)}
                    >
                      <span className="toggle-knob" />
                    </button>
                  </label>
                  <span className="settings-hint">
                    Off by default. A gentle &ldquo;weeks on the air&rdquo; counter on the Journey
                    board — never a daily streak, never a penalty for a break.
                  </span>
                </div>

                <div className="settings-field">
                  <label className="settings-toggle">
                    <span className="settings-label">Beacon — announce presence (CQ)</span>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={form.beacon}
                      className={`toggle${form.beacon ? ' on' : ''}`}
                      onClick={() => updateBool('beacon', !form.beacon)}
                    >
                      <span className="toggle-knob" />
                    </button>
                  </label>
                  <span className="settings-hint">
                    Off = passive (hunt &amp; pounce): Nexus listens and only transmits when you act.
                    On = periodically calls CQ to announce you&apos;re on frequency.
                  </span>
                </div>

                <div className="settings-field">
                  <label className="settings-toggle">
                    <span className="settings-label">IR-HARQ — combine retransmissions</span>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={form.harqEnabled}
                      className={`toggle${form.harqEnabled ? ' on' : ''}`}
                      onClick={() => updateBool('harqEnabled', !form.harqEnabled)}
                    >
                      <span className="toggle-knob" />
                    </button>
                  </label>
                  <span className="settings-hint">
                    On (default) = a weak frame that fails is recovered by joint-combining its
                    retransmissions (RV0+RV1+RV2), and unacknowledged QSO overs escalate redundancy.
                    Off = RV0-only (each frame decoded on its own).
                  </span>
                </div>

                <div className="settings-field">
                  <label className="settings-toggle">
                    <span className="settings-label">CW ID after 73</span>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={form.cwIdAfter73 === true}
                      className={`toggle${form.cwIdAfter73 === true ? ' on' : ''}`}
                      onClick={() => updateBool('cwIdAfter73', form.cwIdAfter73 !== true)}
                    >
                      <span className="toggle-knob" />
                    </button>
                  </label>
                  <span className="settings-hint">
                    Keys your callsign in CW once the final 73 has fully left the air (stock
                    WSJT-X option, default off). Uses the normal CW keying path — PTT + tone —
                    after the FT8 over, never on top of it.
                  </span>
                </div>

                <div className="settings-field">
                  <label className="settings-toggle">
                    <span className="settings-label">Clock check (NTP)</span>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={form.clockCheck}
                      className={`toggle${form.clockCheck ? ' on' : ''}`}
                      onClick={() => updateBool('clockCheck', !form.clockCheck)}
                    >
                      <span className="toggle-knob" />
                    </button>
                  </label>
                  <span className="settings-hint">
                    Periodically check your PC clock against an NTP server and show the offset in the
                    top bar. FT1/DX1 are slot-timed to UTC — keep it within ~0.5 s (NTP / time.is;
                    off-grid: GPS). Turn off for fully-offline operation (no network calls).
                  </span>
                </div>

                <div className="settings-field">
                  <label className="settings-label" htmlFor="station-power">
                    Station power (W)
                  </label>
                  <input
                    id="station-power"
                    className="settings-input"
                    type="number"
                    min="0"
                    step="1"
                    inputMode="decimal"
                    value={form.stationPowerW ?? ''}
                    placeholder="e.g. 100"
                    onChange={(e) => {
                      markDirty()
                      const raw = e.target.value.trim()
                      const num = raw === '' ? null : Number(raw)
                      setForm((prev) =>
                        prev
                          ? {
                              ...prev,
                              stationPowerW:
                                num !== null && Number.isNaN(num) ? prev.stationPowerW : num,
                            }
                          : prev,
                      )
                    }}
                  />
                  <span className="settings-hint">
                    Your transmit power in watts — unlocks the Journey miles-per-watt &amp; QRP feats.
                    Leave blank if unknown.
                  </span>
                </div>
              </div>
            </div>
          </fieldset>
          )}

          {/* ---- Frequencies (working-frequency table overrides) ---- */}
          {tab === 'frequencies' && (
          <fieldset className="settings-section">
            <legend>Working Frequencies</legend>
            <p className="settings-note">
              The dial frequency used when a band/mode is selected. These are{' '}
              <strong>overrides</strong> of the stock WSJT-X working-frequency table — leave the
              list empty to use stock everywhere. An override replaces the stock row for its
              band + mode (e.g. to move FT8 to an alternate sub-band).
            </p>

            <div className="settings-field">
              <span className="settings-label">Standard table (read-only)</span>
              <div className="freq-table">
                <div className="freq-row head">
                  <span className="freq-cell">Band</span>
                  <span className="freq-cell">Mode</span>
                  <span className="freq-cell">Dial (MHz)</span>
                </div>
                {STOCK_WORKING_FREQUENCIES.map((r) => {
                  const ov = overrideByKey.get(`${r.band}|${r.mode}`)
                  return (
                    <div className="freq-row" key={`${r.band}-${r.mode}`}>
                      <span className="freq-cell mono">{r.band}</span>
                      <span className="freq-cell">{r.mode}</span>
                      {ov != null ? (
                        <span
                          className="freq-cell mono freq-override"
                          title={`Your override — stock is ${r.mhz.toFixed(6)} MHz`}
                        >
                          {ov.toFixed(6)}
                          <span className="freq-override-tag">override</span>
                        </span>
                      ) : (
                        <span className="freq-cell mono">{r.mhz.toFixed(6)}</span>
                      )}
                    </div>
                  )
                })}
              </div>
              <span className="settings-hint">
                WSJT-X stock dial frequencies. A row with an active override shows your value
                (highlighted) instead of the stock one.
              </span>
            </div>

            <div className="settings-field">
              <span className="settings-label">Your overrides</span>
              {overrides.length === 0 && (
                <span className="settings-hint">None — the stock table is in effect.</span>
              )}
              {overrides.map((o, i) => {
                const dup = dupKeys.has(`${o.band}|${o.mode}`)
                return (
                  <div className={`freq-edit-row${dup ? ' dup' : ''}`} key={i}>
                    <select
                      className="settings-input"
                      value={o.band}
                      aria-label={`Override ${i + 1} band`}
                      onChange={(e) => updateOverride(i, { band: e.target.value })}
                    >
                      {FREQ_BANDS.map((b) => (
                        <option key={b} value={b}>
                          {b}
                        </option>
                      ))}
                    </select>
                    <select
                      className="settings-input"
                      value={o.mode}
                      aria-label={`Override ${i + 1} mode`}
                      onChange={(e) => updateOverride(i, { mode: e.target.value })}
                    >
                      {FREQ_MODES.map((m) => (
                        <option key={m} value={m}>
                          {m}
                        </option>
                      ))}
                    </select>
                    <input
                      className="settings-input"
                      type="number"
                      inputMode="decimal"
                      min="0"
                      step="0.0001"
                      aria-label={`Override ${i + 1} dial frequency in MHz`}
                      value={mhzDraft && mhzDraft.idx === i ? mhzDraft.text : o.mhz.toFixed(6)}
                      onChange={(e) => {
                        setMhzDraft({ idx: i, text: e.target.value })
                        commitMhz(i, e.target.value)
                      }}
                      onBlur={() => setMhzDraft(null)}
                      autoComplete="off"
                    />
                    <button
                      type="button"
                      className="settings-refresh"
                      onClick={() => removeOverride(i)}
                      aria-label={`Remove the ${o.band} ${o.mode} override`}
                      title="Remove this override"
                    >
                      ✕
                    </button>
                    {dup && (
                      <span className="freq-dup-tag">duplicate band + mode — the last row wins</span>
                    )}
                  </div>
                )
              })}
              <div className="settings-input-row freq-actions">
                <button type="button" className="settings-refresh" onClick={addOverride}>
                  Add override
                </button>
                <button
                  type="button"
                  className="settings-refresh"
                  onClick={resetOverrides}
                  disabled={overrides.length === 0}
                >
                  Reset to standard
                </button>
              </div>
              <span className="settings-hint">
                MHz is the dial (suppressed-carrier) frequency. Save to apply — band switches
                then use your value for that band + mode.
              </span>
            </div>
          </fieldset>
          )}

          {/* ---- Alerts ---- */}
          {tab === 'alerts' && (
          <>
          <fieldset className="settings-section">
            <legend>Alerts</legend>
            <div className="settings-grid">
              <div className="settings-field">
                <label className="settings-toggle">
                  <span className="settings-label">My call</span>
                  <button
                    type="button"
                    role="switch"
                    aria-checked={form.alertMyCall}
                    className={`toggle${form.alertMyCall ? ' on' : ''}`}
                    onClick={() => updateBool('alertMyCall', !form.alertMyCall)}
                  >
                    <span className="toggle-knob" />
                  </button>
                </label>
                <span className="settings-hint">Beep + flash when someone directs a call at you.</span>
              </div>

              <div className="settings-field">
                <label className="settings-toggle">
                  <span className="settings-label">CQ calls</span>
                  <button
                    type="button"
                    role="switch"
                    aria-checked={form.alertCq}
                    className={`toggle${form.alertCq ? ' on' : ''}`}
                    onClick={() => updateBool('alertCq', !form.alertCq)}
                  >
                    <span className="toggle-knob" />
                  </button>
                </label>
                <span className="settings-hint">Alert on any decoded CQ. Off by default — CQs are constant.</span>
              </div>

              <div className="settings-field">
                <label className="settings-toggle">
                  <span className="settings-label">New DXCC / grid</span>
                  <button
                    type="button"
                    role="switch"
                    aria-checked={form.alertNew}
                    className={`toggle${form.alertNew ? ' on' : ''}`}
                    onClick={() => updateBool('alertNew', !form.alertNew)}
                  >
                    <span className="toggle-knob" />
                  </button>
                </label>
                <span className="settings-hint">
                  Loudly alert on a new DXCC entity (a “new one”) or a new grid — the things worth
                  chasing. Does NOT alert on every decode.
                </span>
              </div>

            </div>
            <div className="settings-watchlist-block">
              <span className="settings-label">Watch list</span>
              <WatchlistPanel />
            </div>
          </fieldset>

          {/* ---- Macros ---- */}
          <fieldset className="settings-section">
            <legend>Quick-reply Macros</legend>
            <div className="settings-grid">
              <label className="settings-field">
                <span className="settings-label">Chat</span>
                <input
                  className="settings-input"
                  type="text"
                  value={form.macros.chat.join(', ')}
                  onChange={(e) => updateMacros('chat', e.target.value)}
                  autoComplete="off"
                  spellCheck={false}
                />
                <span className="settings-hint">Comma-separated chips for Chat.</span>
              </label>
              <label className="settings-field">
                <span className="settings-label">QSO</span>
                <input
                  className="settings-input"
                  type="text"
                  value={form.macros.qso.join(', ')}
                  onChange={(e) => updateMacros('qso', e.target.value)}
                  autoComplete="off"
                  spellCheck={false}
                />
                <span className="settings-hint">Chips for sequenced QSOs.</span>
              </label>
              <label className="settings-field">
                <span className="settings-label">Band / CQ</span>
                <input
                  className="settings-input"
                  type="text"
                  value={form.macros.band.join(', ')}
                  onChange={(e) => updateMacros('band', e.target.value)}
                  autoComplete="off"
                  spellCheck={false}
                />
                <span className="settings-hint">Open broadcasts — the Call CQ launchpad + band feed.</span>
              </label>
            </div>

            <div className="settings-field cw-macro-editor">
              <span className="settings-label">CW cockpit F-keys</span>
              {/* Named macro profiles — a rotating operator switches sets here (or in one
                  click from the CW cockpit bar). The grid below edits the ACTIVE profile. */}
              <div className="cw-macro-row">
                <select
                  className="settings-input"
                  value={activeCwIdx}
                  onChange={(e) => selectCwProfile(Number(e.target.value))}
                  aria-label="Active CW macro profile"
                >
                  {cwProfiles.map((p, i) => (
                    <option key={i} value={i}>
                      {p.name || `Profile ${i + 1}`}
                    </option>
                  ))}
                </select>
                <button type="button" className="settings-refresh" onClick={addCwProfile}>
                  New
                </button>
                <button type="button" className="settings-refresh" onClick={renameCwProfile}>
                  Rename
                </button>
                <button
                  type="button"
                  className="settings-refresh danger"
                  onClick={deleteCwProfile}
                  disabled={cwProfiles.length <= 1}
                  title={cwProfiles.length <= 1 ? 'Keep at least one profile' : 'Delete this profile'}
                >
                  Delete
                </button>
              </div>
              {!activeCwMacros.length ? (
                <div className="cw-macro-row">
                  <span className="settings-hint">
                    Using the built-in F1–F8 set. Customize to make them your own (labels +
                    templates; tokens: {'{MYCALL} {RST} {NAME}'} and ! = the worked call).
                  </span>
                  <button
                    type="button"
                    className="settings-refresh"
                    onClick={() => setCwMacros(CW_MACRO_DEFAULTS.map((m) => ({ ...m })))}
                  >
                    Customize
                  </button>
                </div>
              ) : (
                <>
                  {activeCwMacros.map((m, i) => (
                    <div key={m.key} className="cw-macro-row">
                      <span className="cw-macro-key" title={CW_MACRO_ROLES[m.key] ?? ''}>
                        {m.key}
                      </span>
                      <span className="cw-macro-role">{CW_MACRO_ROLES[m.key] ?? ''}</span>
                      <input
                        className="settings-input cw-macro-label"
                        type="text"
                        value={m.label}
                        onChange={(e) => updateCwMacro(i, 'label', e.target.value)}
                        aria-label={`${m.key} label`}
                        autoComplete="off"
                        spellCheck={false}
                      />
                      <input
                        className="settings-input cw-macro-text"
                        type="text"
                        value={m.text}
                        onChange={(e) => updateCwMacro(i, 'text', e.target.value)}
                        aria-label={`${m.key} text`}
                        autoComplete="off"
                        spellCheck={false}
                      />
                    </div>
                  ))}
                  <div className="cw-macro-row">
                    <span className="settings-hint">
                      Tokens: {'{MYCALL} {RST} {NAME}'} · ! = the worked call (auto-filled from
                      the copilot / roster click). Each key KEEPS its role — the Guided
                      copilot's next-step highlight follows the role, so customized text
                      still rolls through F1→F2→F3→F4 exactly as before. Keep the ! token
                      wherever you want the other station's call inserted. Save to apply.
                    </span>
                    <button
                      type="button"
                      className="settings-refresh"
                      onClick={() => setCwMacros([])}
                    >
                      Reset to defaults
                    </button>
                  </div>
                </>
              )}
            </div>
          </fieldset>
          </>
          )}

          {/* ---- Network integrations ---- */}
          {tab === 'connections' && (
          <fieldset className="settings-section">
            <legend>Integrations &amp; Feeds</legend>
            <div className="settings-featgroup">
              <span className="settings-featgroup-title">Local APIs &amp; Loggers</span>
              <div className="settings-grid">
                <div className="settings-field">
                  <label className="settings-toggle">
                    <span className="settings-label">WSJT-X UDP API</span>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={form.wsjtxUdp}
                      className={`toggle${form.wsjtxUdp ? ' on' : ''}`}
                      onClick={() => updateBool('wsjtxUdp', !form.wsjtxUdp)}
                    >
                      <span className="toggle-knob" />
                    </button>
                  </label>
                  <span className="settings-hint">for JTAlert / GridTracker / loggers</span>
                </div>

                <label className="settings-field">
                  <span className="settings-label">UDP Address</span>
                  <input
                    className="settings-input"
                    type="text"
                    value={form.wsjtxUdpAddr}
                    placeholder="127.0.0.1:2237"
                    onChange={(e) => update('wsjtxUdpAddr', e.target.value)}
                    disabled={!form.wsjtxUdp}
                    autoComplete="off"
                    spellCheck={false}
                  />
                  <span className="settings-hint">host:port for the UDP feed</span>
                </label>

                <div className="settings-field">
                  <label className="settings-toggle">
                    <span className="settings-label">Ham Radio Deluxe logging</span>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={form.hrdLogging}
                      className={`toggle${form.hrdLogging ? ' on' : ''}`}
                      onClick={() => updateBool('hrdLogging', !form.hrdLogging)}
                    >
                      <span className="toggle-knob" />
                    </button>
                  </label>
                  <span className="settings-hint">
                    push each QSO to HRD Logbook over its QSO-Forwarding UDP port (HRD must be running;
                    don't also run JTAlert/QSO Relay into HRD or you'll double-log)
                  </span>
                </div>

                <label className="settings-field">
                  <span className="settings-label">HRD UDP Address</span>
                  <input
                    className="settings-input"
                    type="text"
                    value={form.hrdUdpAddr}
                    placeholder="127.0.0.1:2333"
                    onChange={(e) => update('hrdUdpAddr', e.target.value)}
                    disabled={!form.hrdLogging}
                    autoComplete="off"
                    spellCheck={false}
                  />
                  <span className="settings-hint">HRD QSO-Forwarding host:port (default 127.0.0.1:2333)</span>
                </label>

                <label className="settings-field">
                  <span className="settings-label">Companion UDP address</span>
                  <input
                    className="settings-input"
                    value={form.companionAddr ?? ''}
                    onChange={(e) => update('companionAddr', e.target.value)}
                    placeholder="127.0.0.1:2237"
                    spellCheck={false}
                  />
                  <span className="settings-hint">
                    Where Nexus listens for WSJT-X/JTDX in Companion source mode.
                  </span>
                </label>

                <div className="settings-field">
                  <label className="settings-toggle">
                    <span className="settings-label">Write ALL.TXT decode log</span>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={form.writeAllTxt}
                      className={`toggle${form.writeAllTxt ? ' on' : ''}`}
                      onClick={() => updateBool('writeAllTxt', !form.writeAllTxt)}
                    >
                      <span className="toggle-knob" />
                    </button>
                  </label>
                  <span className="settings-hint">WSJT-X-format ALL.TXT for GridTracker / loggers to tail</span>
                </div>

                <div className="settings-field">
                  <label className="settings-toggle">
                    <span className="settings-label">Save a WAV per logged QSO</span>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={form.saveQsoWav}
                      className={`toggle${form.saveQsoWav ? ' on' : ''}`}
                      onClick={() => updateBool('saveQsoWav', !form.saveQsoWav)}
                    >
                      <span className="toggle-knob" />
                    </button>
                  </label>
                  <span className="settings-hint">Auto-records the last ~60 s of RX audio to the recordings folder on log</span>
                </div>

                <label className="settings-field">
                  <span className="settings-label">Save received audio (.wav per period)</span>
                  <select
                    value={form.saveWav || 'none'}
                    onChange={(e) => update('saveWav', e.target.value)}
                  >
                    <option value="none">None (default)</option>
                    <option value="decodes">Save periods with decodes</option>
                    <option value="all">Save all periods</option>
                  </select>
                  <span className="settings-hint">
                    WAVs land in recordings/periods (12 kHz mono, ~360 KB each). "All" writes
                    ~2 GB/day of continuous monitoring — use for decoder debugging, not always-on.
                  </span>
                </label>
              </div>
            </div>

            <div className="settings-featgroup">
              <span className="settings-featgroup-title">Spot Sources</span>
              <div className="settings-grid">
                <div className="settings-field">
                  <label className="settings-toggle">
                    <span className="settings-label">PSK Reporter</span>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={form.pskreporter}
                      className={`toggle${form.pskreporter ? ' on' : ''}`}
                      onClick={() => updateBool('pskreporter', !form.pskreporter)}
                    >
                      <span className="toggle-knob" />
                    </button>
                  </label>
                  <span className="settings-hint">upload spots to the global map</span>
                </div>

                <div className="settings-field">
                  <label className="settings-toggle">
                    <span className="settings-label">DX Cluster / RBN spots</span>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={!!form.clusterEnabled}
                      className={`toggle${form.clusterEnabled ? ' on' : ''}`}
                      onClick={() => updateBool('clusterEnabled', !form.clusterEnabled)}
                    >
                      <span className="toggle-knob" />
                    </button>
                  </label>
                  <span className="settings-hint">
                    Surface "new ones" from the Reverse Beacon Network on the Needed board + Connect.
                    Takes effect on restart.
                  </span>
                </div>

                <div className="settings-field">
                  <span className="settings-label">Phone/SSB cluster nodes</span>
                  {(form.clusterHosts ?? []).length === 0 ? (
                    <span className="settings-hint cluster-node-empty">
                      No nodes — add one below to get SSB/phone needs (RBN only carries CW + digital).
                    </span>
                  ) : (
                    (form.clusterHosts ?? []).map((host, i) => (
                      <div key={i} className="cluster-node-row">
                        <input
                          className="settings-input"
                          value={host}
                          onChange={(e) =>
                            mutateClusterHosts((hs) => hs.map((h, j) => (j === i ? e.target.value : h)))
                          }
                          placeholder="ve7cc.net:23"
                          spellCheck={false}
                        />
                        <button
                          type="button"
                          className="cluster-node-remove"
                          title="Remove this cluster node"
                          aria-label={`Remove ${host || 'node'}`}
                          onClick={() => mutateClusterHosts((hs) => hs.filter((_, j) => j !== i))}
                        >
                          ✕
                        </button>
                      </div>
                    ))
                  )}
                  <div className="cluster-node-add">
                    <select
                      className="settings-input"
                      value=""
                      onChange={(e) => {
                        const host = e.target.value
                        if (!host) return
                        mutateClusterHosts((hs) =>
                          hs.some((h) => h.trim().toLowerCase() === host.toLowerCase())
                            ? hs
                            : [...hs, host],
                        )
                      }}
                    >
                      <option value="">+ Add a known node…</option>
                      {CLUSTER_PRESETS.map((p) => (
                        <option key={p.host} value={p.host}>
                          {p.label}
                        </option>
                      ))}
                    </select>
                    <button
                      type="button"
                      className="cluster-node-add-blank"
                      title="Add a custom node row"
                      onClick={() => mutateClusterHosts((hs) => [...hs, ''])}
                    >
                      + Custom
                    </button>
                  </div>
                  <span className="settings-hint">
                    We connect to ALL listed nodes and union their human SSB/phone spots — more
                    nodes = wider phone coverage (RBN CW + digital connect automatically; RBN
                    endpoints are ignored here). An added node connects on the next Save; removing
                    one takes effect on restart.
                  </span>
                </div>
              </div>
            </div>

            <div className="settings-featgroup">
              <span className="settings-featgroup-title">Propagation</span>
              <div className="settings-grid">
                <div className="settings-field">
                  <label className="settings-toggle">
                    <span className="settings-label">Near-region opening watch</span>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={form.openingRegional}
                      className={`toggle${form.openingRegional ? ' on' : ''}`}
                      onClick={() => updateBool('openingRegional', !form.openingRegional)}
                    >
                      <span className="toggle-knob" />
                    </button>
                  </label>
                  <span className="settings-hint">
                    Watch VHF/10 m activity near your QTH (not just your own contacts) so openings flag "open
                    around you" before you've worked anyone. Takes effect on restart.
                  </span>
                </div>

                <label className="settings-field">
                  <span className="settings-label">Prediction engine</span>
                  <select
                    value={form.propEngine || 'heuristic'}
                    onChange={(e) => update('propEngine', e.target.value)}
                  >
                    <option value="heuristic">Modelled (fast heuristic)</option>
                    <option value="p533">ITU-R P.533 (full physics)</option>
                  </select>
                  <span className="settings-hint">
                    Drives the per-station path outlook + 24h band×hour grid. P.533 is the real
                    circuit-reliability method (validated against the ITU reference; ~0.1 s per
                    prediction, uses your station power). Live spots always win over any model.
                  </span>
                </label>
              </div>
              <SettingsGroup title="Antenna gain (advanced)" defaultOpen={false}>
                <div className="settings-field">
                  <span className="settings-label">Antenna gain (dBi) — TX / RX</span>
                  <div className="settings-inline-pair">
                    {(['antTxGainDbi', 'antRxGainDbi'] as const).map((k) => (
                      <input
                        key={k}
                        className="settings-input"
                        type="number"
                        step="0.5"
                        min="-10"
                        max="30"
                        inputMode="decimal"
                        aria-label={k === 'antTxGainDbi' ? 'TX antenna gain (dBi)' : 'RX antenna gain (dBi)'}
                        value={form[k] ?? 0}
                        onChange={(e) => {
                          const num = Number(e.target.value)
                          if (!Number.isNaN(num)) updateNum(k, num)
                        }}
                      />
                    ))}
                  </div>
                  <span className="settings-hint">
                    Used by the P.533 link budget only. 0 = a simple wire/vertical (isotropic);
                    a 3-element yagi ≈ 6–8. Honest v1: a plain dB shift — no pattern or
                    takeoff-angle modelling, and the fast heuristic ignores it.
                  </span>
                </div>
              </SettingsGroup>
            </div>
          </fieldset>
          )}

          {/* ---- Confirmations (LoTW / eQSL / QRZ / ClubLog accounts) ---- */}
          {tab === 'confirmations' && (
          <>
          <fieldset className="settings-section">
            <legend>LoTW users list</legend>
            <div className="settings-field">
              <div className="lotw-users-row">
                <button
                  type="button"
                  className="settings-test-btn"
                  disabled={lotwFetching}
                  onClick={() => {
                    setLotwFetching(true)
                    fetchLotwUsers()
                      .then((st) => {
                        setLotwUsers(st)
                        pushToast(
                          `LoTW list loaded — ${st.count.toLocaleString()} calls`,
                          'success',
                          5000,
                        )
                      })
                      .catch((e) =>
                        pushToast(
                          `LoTW list fetch failed: ${e instanceof Error ? e.message : e}`,
                          'error',
                        ),
                      )
                      .finally(() => setLotwFetching(false))
                  }}
                >
                  {lotwFetching ? 'Fetching…' : 'Fetch now'}
                </button>
                <span className="settings-hint">
                  {lotwUsers && lotwUsers.count > 0
                    ? `${lotwUsers.count.toLocaleString()} calls · fetched ${new Date(lotwUsers.fetchedAt * 1000).toISOString().slice(0, 10)}`
                    : 'Not fetched yet — decode lists gain an L mark on calls that upload to LoTW.'}
                </span>
              </div>
              <label className="settings-label" htmlFor="lotw-max-age" style={{ marginTop: 8 }}>
                Count as a LoTW user if uploaded within (days)
              </label>
              <input
                id="lotw-max-age"
                className="settings-input"
                type="number"
                min="30"
                max="3650"
                step="1"
                style={{ width: '7em' }}
                value={form.lotwMaxAgeDays ?? 365}
                onChange={(e) => {
                  const n = Number(e.target.value)
                  if (!Number.isNaN(n)) updateNum('lotwMaxAgeDays', n)
                }}
              />
              <span className="settings-hint">
                ARRL's activity list updates weekly — refetching more often just returns
                "unchanged". Manual fetch by design (WSJT-X convention).
              </span>
            </div>
          </fieldset>
          <fieldset className="settings-section">
            <legend>Connections</legend>
            <div className="conn-status-grid">
              {creds.map((c) => (
                <div key={c.connector} className="conn-status-row">
                  <span className={`conn-dot ${c.stored ? 'on' : 'off'}`} aria-hidden="true" />
                  <span className="conn-name">{c.connector}</span>
                  <span className="conn-id">{c.identity || '—'}</span>
                  <span className={`conn-state ${c.stored ? 'on' : 'off'}`}>
                    {c.stored ? 'credential stored' : 'no credential'}
                  </span>
                  {c.connector === 'QRZ Logbook' && (
                    <button
                      type="button"
                      className="settings-test-btn"
                      onClick={runQrzTest}
                      disabled={qrzTest.state === 'testing'}
                      title="Round-trips the QRZ Logbook API (ACTION=STATUS) — proves the key works without logging anything"
                    >
                      {qrzTest.state === 'testing' ? 'Testing…' : 'Test'}
                    </button>
                  )}
                </div>
              ))}
            </div>
            {qrzTest.state !== 'idle' && qrzTest.state !== 'testing' && (
              <p className={`conn-test-result ${qrzTest.state}`}>
                {qrzTest.state === 'ok' ? '✓ QRZ Logbook reachable: ' : '✗ QRZ test failed: '}
                {qrzTest.msg}
                {qrzTest.state === 'fail' && (
                  <>
                    {' '}
                    (Uploads need the per-logbook <strong>API key</strong> from
                    logbook.qrz.com ▸ Settings ▸ API — not your QRZ password.)
                  </>
                )}
              </p>
            )}
            <div className="conn-log">
              <div className="conn-log-head">
                <span>Connection log</span>
                <span className="settings-hint">every save, sync, push, and failure lands here</span>
              </div>
              {connLog.length === 0 ? (
                <p className="conn-log-empty">
                  No events yet this session — save a credential or run a sync and it shows here.
                </p>
              ) : (
                <ul className="conn-log-list">
                  {connLog.slice(0, 40).map((e, i) => (
                    <li key={`${e.tsUnix}-${i}`} className={`conn-ev ${e.level}`}>
                      <span className="conn-ev-time">
                        {new Date(e.tsUnix * 1000).toLocaleTimeString([], { hour12: false })}
                      </span>
                      <span className="conn-ev-name">{e.connector}</span>
                      <span className="conn-ev-msg">{e.message}</span>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          </fieldset>
          <fieldset className="settings-section">
            <legend>Confirmations</legend>
            <div className="settings-featgroup">
              <span className="settings-featgroup-title">LoTW</span>
              <div className="settings-grid">
                <label className="settings-field">
                  <span className="settings-label">LoTW username</span>
                  <input
                    className="settings-input"
                    type="text"
                    value={form.lotwUsername}
                    placeholder="your LoTW account login"
                    onChange={(e) => update('lotwUsername', e.target.value)}
                    autoComplete="off"
                    spellCheck={false}
                  />
                  <span className="settings-hint">
                    Often your callsign, but not always — use your LoTW account login. Save settings to apply.
                  </span>
                </label>

                <label className="settings-field">
                  <span className="settings-label">LoTW password</span>
                  <div className="settings-input-row">
                    <input
                      className="settings-input"
                      type="password"
                      value={lotwPw}
                      placeholder="LoTW website password"
                      onChange={(e) => setLotwPw(e.target.value)}
                      autoComplete="off"
                      spellCheck={false}
                    />
                    <button
                      type="button"
                      className="settings-refresh"
                      onClick={onSaveLotwPassword}
                      disabled={!lotwPw}
                    >
                      Set
                    </button>
                    <button
                      type="button"
                      className="settings-refresh"
                      onClick={onForgetLotwPassword}
                      title="Remove the stored password from the system keychain"
                    >
                      Forget
                    </button>
                  </div>
                  <span className="settings-hint">
                    Your LoTW <strong>website</strong> password (not your TQSL certificate password). Stored in
                    the OS keychain, never on disk; not shown again after you click Set.
                  </span>
                </label>

                <div className="settings-field">
                  <span className="settings-label">LoTW sync</span>
                  <div className="settings-input-row">
                    <button
                      type="button"
                      className="settings-refresh"
                      onClick={onSyncLotw}
                      disabled={lotwSyncing || !form.lotwUsername.trim()}
                    >
                      {lotwSyncing ? 'Syncing…' : 'Sync LoTW now'}
                    </button>
                  </div>
                  <span className="settings-hint">
                    Pulls new confirmations into your log and marks which of your uploads LoTW now holds on file
                    (so they read “waiting on the other op,” not “never uploaded”). The first sync pulls your whole
                    history (can be slow); later syncs are incremental.
                  </span>
                </div>

                <label className="settings-field">
                  <span className="settings-label">LoTW Station Location</span>
                  <input
                    className="settings-input"
                    type="text"
                    value={form.lotwStationLocation}
                    placeholder="exact TQSL Station Location name"
                    onChange={(e) => update('lotwStationLocation', e.target.value)}
                    autoComplete="off"
                    spellCheck={false}
                  />
                  <span className="settings-hint">
                    For <strong>uploading</strong> to LoTW (the "Upload to LoTW" button in the Logbook). Signing is
                    done by your installed <strong>TQSL</strong> against this named Station Location — set it up in
                    TQSL first; the name must match exactly. No certificate or password is stored by Nexus.
                  </span>
                </label>

                <label className="settings-field">
                  <span className="settings-label">Sign from ADIF location (travelers)</span>
                  <span className="settings-input-row">
                    <input
                      type="checkbox"
                      checked={!!form.lotwUseAdifLocation}
                      onChange={(e) => updateBool('lotwUseAdifLocation', e.target.checked)}
                      aria-label="Sign LoTW uploads from the ADIF location"
                    />
                    <span className="settings-hint">
                      Turn on if you set TQSL to <em>"use the location in the ADIF file"</em> and don't create
                      named Station Locations (handy if you travel). Nexus then stamps your call + grid
                      (STATION_CALLSIGN / MY_GRIDSQUARE) into the upload and omits the <code>-l</code> argument,
                      so TQSL signs from those and the Station Location above isn't required.{' '}
                      <strong>The whole batch is signed from your current grid above</strong>, so if you operate
                      from more than one location, upload <em>before</em> you move — otherwise earlier contacts
                      are signed with the new grid.
                    </span>
                  </span>
                </label>

                <label className="settings-field">
                  <span className="settings-label">TQSL path (optional)</span>
                  <input
                    className="settings-input"
                    type="text"
                    value={form.tqslPath}
                    placeholder="auto-detect (leave blank)"
                    onChange={(e) => update('tqslPath', e.target.value)}
                    autoComplete="off"
                    spellCheck={false}
                  />
                  <span className="settings-hint">
                    Only if TQSL is installed somewhere non-standard; otherwise leave blank to auto-detect.
                  </span>
                </label>
              </div>
            </div>
            <div className="settings-featgroup">
              <span className="settings-featgroup-title">eQSL</span>
              <div className="settings-grid">
                <label className="settings-field">
                  <span className="settings-label">eQSL username</span>
                  <input
                    className="settings-input"
                    type="text"
                    value={form.eqslUsername}
                    placeholder="your eQSL.cc account login"
                    onChange={(e) => update('eqslUsername', e.target.value)}
                    autoComplete="off"
                    spellCheck={false}
                  />
                  <span className="settings-hint">
                    Your eQSL.cc login (often your callsign). Save settings to apply.
                  </span>
                </label>

                <label className="settings-field">
                  <span className="settings-label">eQSL password</span>
                  <div className="settings-input-row">
                    <input
                      className="settings-input"
                      type="password"
                      value={eqslPw}
                      placeholder="eQSL.cc account password"
                      onChange={(e) => setEqslPw(e.target.value)}
                      autoComplete="off"
                      spellCheck={false}
                    />
                    <button
                      type="button"
                      className="settings-refresh"
                      onClick={onSaveEqslPassword}
                      disabled={!eqslPw}
                    >
                      Set
                    </button>
                    <button
                      type="button"
                      className="settings-refresh"
                      onClick={onForgetEqslPassword}
                      title="Remove the stored password from the system keychain"
                    >
                      Forget
                    </button>
                  </div>
                  <span className="settings-hint">
                    Stored in the OS keychain, never on disk; not shown again after you click Set.
                  </span>
                </label>

                <div className="settings-field">
                  <span className="settings-label">eQSL confirmations</span>
                  <div className="settings-input-row">
                    <button
                      type="button"
                      className="settings-refresh"
                      onClick={onSyncEqsl}
                      disabled={eqslSyncing || !form.eqslUsername.trim()}
                    >
                      {eqslSyncing ? 'Syncing…' : 'Sync eQSL now'}
                    </button>
                  </div>
                  <span className="settings-hint">
                    Download eQSL confirmations into your log. These count as confirmations but{' '}
                    <strong>not</strong> for DXCC/WAS (ARRL doesn't accept eQSL) — a separate tier.
                  </span>
                </div>

                <div className="settings-field">
                  <label className="settings-toggle">
                    <span className="settings-label">Auto-upload QSOs to eQSL</span>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={form.eqslUpload}
                      className={`toggle${form.eqslUpload ? ' on' : ''}`}
                      onClick={() => updateBool('eqslUpload', !form.eqslUpload)}
                    >
                      <span className="toggle-knob" />
                    </button>
                  </label>
                  <span className="settings-hint">
                    Upload each logged QSO to eQSL.cc as you log it (needs the eQSL username + password above).
                  </span>
                </div>
              </div>
            </div>
            <div className="settings-featgroup">
              <span className="settings-featgroup-title">QRZ</span>
              <div className="settings-grid">
                <label className="settings-field">
                  <span className="settings-label">QRZ username</span>
                  <input
                    className="settings-input"
                    type="text"
                    value={form.qrzUsername}
                    placeholder="your QRZ.com account login"
                    onChange={(e) => update('qrzUsername', e.target.value)}
                    autoComplete="off"
                    spellCheck={false}
                  />
                  <span className="settings-hint">
                    Used to look up a callsign's name + grid when logging. Save settings to apply.
                  </span>
                </label>

                <label className="settings-field">
                  <span className="settings-label">QRZ password</span>
                  <div className="settings-input-row">
                    <input
                      className="settings-input"
                      type="password"
                      value={qrzPw}
                      placeholder="QRZ.com account password"
                      onChange={(e) => setQrzPw(e.target.value)}
                      autoComplete="off"
                      spellCheck={false}
                    />
                    <button
                      type="button"
                      className="settings-refresh"
                      onClick={onSaveQrzPassword}
                      disabled={!qrzPw}
                    >
                      Set
                    </button>
                    <button
                      type="button"
                      className="settings-refresh"
                      onClick={onForgetQrzPassword}
                      title="Remove the stored password from the system keychain"
                    >
                      Forget
                    </button>
                  </div>
                  <span className="settings-hint">
                    Your QRZ.com login password — <strong>this is what powers callbook lookups</strong>{' '}
                    (name, QTH, grid), and it is separate from the Logbook API key below (that key only
                    uploads QSOs). Stored in the OS keychain, never on disk. Grid &amp; state need a QRZ
                    XML subscription; free accounts return only name/address/country.
                  </span>
                </label>

                <label className="settings-field">
                  <span className="settings-label">QRZ Logbook API key</span>
                  <div className="settings-input-row">
                    <input
                      className="settings-input"
                      type="password"
                      value={qrzKey}
                      placeholder="from your QRZ logbook settings page"
                      onChange={(e) => setQrzKey(e.target.value)}
                      autoComplete="off"
                      spellCheck={false}
                    />
                    <button
                      type="button"
                      className="settings-refresh"
                      onClick={onSaveQrzLogbookKey}
                      disabled={!qrzKey}
                    >
                      Set
                    </button>
                    <button
                      type="button"
                      className="settings-refresh"
                      onClick={onForgetQrzLogbookKey}
                      title="Remove the stored Logbook key from the system keychain"
                    >
                      Forget
                    </button>
                  </div>
                  <span className="settings-hint">
                    A <strong>separate</strong> key (not the login password) from your QRZ logbook's settings
                    page — used to upload logged QSOs.
                  </span>
                </label>

                <div className="settings-field">
                  <label className="settings-toggle">
                    <span className="settings-label">Auto-upload QSOs to QRZ</span>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={form.qrzLogbookUpload}
                      className={`toggle${form.qrzLogbookUpload ? ' on' : ''}`}
                      onClick={() => updateBool('qrzLogbookUpload', !form.qrzLogbookUpload)}
                    >
                      <span className="toggle-knob" />
                    </button>
                  </label>
                  <span className="settings-hint">
                    Push each logged QSO to your QRZ logbook (needs the Logbook API key above).
                  </span>
                </div>
              </div>
            </div>
            <div className="settings-featgroup">
              <span className="settings-featgroup-title">HamQTH</span>
              <div className="settings-grid">
                <label className="settings-field">
                  <span className="settings-label">HamQTH username</span>
                  <input
                    className="settings-input"
                    type="text"
                    value={form.hamqthUsername}
                    placeholder="your HamQTH.com account login"
                    onChange={(e) => update('hamqthUsername', e.target.value)}
                    autoComplete="off"
                    spellCheck={false}
                  />
                  <span className="settings-hint">
                    A <strong>free</strong> callbook used as a fallback when QRZ isn't configured or has
                    no match — a HamQTH account returns name, grid &amp; US state at no charge. Save
                    settings to apply.
                  </span>
                </label>

                <label className="settings-field">
                  <span className="settings-label">HamQTH password</span>
                  <div className="settings-input-row">
                    <input
                      className="settings-input"
                      type="password"
                      value={hamqthPw}
                      placeholder="HamQTH.com account password"
                      onChange={(e) => setHamqthPw(e.target.value)}
                      autoComplete="off"
                      spellCheck={false}
                    />
                    <button
                      type="button"
                      className="settings-refresh"
                      onClick={onSaveHamqthPassword}
                      disabled={!hamqthPw}
                    >
                      Set
                    </button>
                    <button
                      type="button"
                      className="settings-refresh"
                      onClick={onForgetHamqthPassword}
                      title="Remove the stored password from the system keychain"
                    >
                      Forget
                    </button>
                  </div>
                  <span className="settings-hint">
                    Stored in the OS keychain, never on disk.
                  </span>
                </label>
              </div>
            </div>
            <div className="settings-featgroup">
              <span className="settings-featgroup-title">ClubLog</span>
              <div className="settings-grid">
                <label className="settings-field">
                  <span className="settings-label">ClubLog email</span>
                  <input
                    className="settings-input"
                    type="text"
                    value={form.clublogEmail}
                    placeholder="your ClubLog account email (not a callsign)"
                    onChange={(e) => update('clublogEmail', e.target.value)}
                    autoComplete="off"
                    spellCheck={false}
                  />
                  <span className="settings-hint">Your ClubLog login email. Save settings to apply.</span>
                </label>

                <label className="settings-field">
                  <span className="settings-label">ClubLog callsign</span>
                  <input
                    className="settings-input"
                    type="text"
                    value={form.clublogCallsign}
                    placeholder="defaults to your callsign"
                    onChange={(e) => update('clublogCallsign', e.target.value)}
                    autoComplete="off"
                    spellCheck={false}
                  />
                  <span className="settings-hint">The ClubLog logbook to upload into (empty = your callsign).</span>
                </label>

                <label className="settings-field">
                  <span className="settings-label">ClubLog app-password</span>
                  <div className="settings-input-row">
                    <input
                      className="settings-input"
                      type="password"
                      value={clublogPw}
                      placeholder="a ClubLog Application Password"
                      onChange={(e) => setClublogPw(e.target.value)}
                      autoComplete="off"
                      spellCheck={false}
                    />
                    <button
                      type="button"
                      className="settings-refresh"
                      onClick={onSaveClublogPassword}
                      disabled={!clublogPw}
                    >
                      Set
                    </button>
                    <button
                      type="button"
                      className="settings-refresh"
                      onClick={onForgetClublogPassword}
                      title="Remove the stored ClubLog password from the system keychain"
                    >
                      Forget
                    </button>
                  </div>
                  <span className="settings-hint">
                    Use a ClubLog <strong>Application Password</strong> (Settings → App Passwords), not your main
                    password. Stored in the OS keychain.
                  </span>
                </label>

                <label className="settings-field">
                  <span className="settings-label">ClubLog API key (application-level)</span>
                  <input
                    className="settings-input"
                    type="text"
                    value={form.clublogApiKey}
                    placeholder="blank = use the key bundled with this build (if any)"
                    onChange={(e) => update('clublogApiKey', e.target.value)}
                    autoComplete="off"
                    spellCheck={false}
                  />
                  <span className="settings-hint">
                    This is the <strong>application</strong> credential, not yours — official installer builds
                    bundle one, and you only need email + app-password above. Building from source? Request a
                    free key at clublog.org/requestapikey.php and paste it here (open-source can't ship one —
                    ClubLog auto-revokes published keys).
                  </span>
                </label>

                <div className="settings-field">
                  <label className="settings-toggle">
                    <span className="settings-label">Auto-upload QSOs to ClubLog</span>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={form.clublogUpload}
                      className={`toggle${form.clublogUpload ? ' on' : ''}`}
                      onClick={() => updateBool('clublogUpload', !form.clublogUpload)}
                    >
                      <span className="toggle-knob" />
                    </button>
                  </label>
                  <span className="settings-hint">
                    Push each logged QSO to ClubLog in real time (needs the email + app-password above; official builds bundle the API key).
                  </span>
                </div>
              </div>
            </div>
            <div className="settings-featgroup">
              <span className="settings-featgroup-title">HRDLog</span>
              <div className="settings-grid">
                <label className="settings-field">
                  <span className="settings-label">HRDLog.net upload code</span>
                  <div className="settings-input-row">
                    <input
                      className="settings-input"
                      type="password"
                      value={hrdlogCode}
                      placeholder="your hrdlog.net upload code"
                      onChange={(e) => setHrdlogCodeField(e.target.value)}
                      autoComplete="off"
                      spellCheck={false}
                    />
                    <button
                      type="button"
                      className="settings-refresh"
                      onClick={onSaveHrdlogCode}
                      disabled={!hrdlogCode}
                    >
                      Set
                    </button>
                    <button
                      type="button"
                      className="settings-refresh"
                      onClick={onForgetHrdlogCode}
                      title="Remove the stored HRDLog.net code from the system keychain"
                    >
                      Forget
                    </button>
                  </div>
                  <span className="settings-hint">
                    The upload code from your HRDLog.net account (Options → your code). Uploads log under your
                    station callsign. Stored in the OS keychain. This is the online HRDLog.net service — separate
                    from the HRD Logbook UDP push under Logging.
                  </span>
                </label>

                <div className="settings-field">
                  <label className="settings-toggle">
                    <span className="settings-label">Auto-upload QSOs to HRDLog.net</span>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={form.hrdlogUpload}
                      className={`toggle${form.hrdlogUpload ? ' on' : ''}`}
                      onClick={() => updateBool('hrdlogUpload', !form.hrdlogUpload)}
                    >
                      <span className="toggle-knob" />
                    </button>
                  </label>
                  <span className="settings-hint">
                    Push each logged QSO to HRDLog.net (needs the upload code above). HRDLog.net is a live-logging
                    and awards site — it is <strong>not</strong> an ARRL confirmation source, so an upload here
                    never earns DXCC/WAS credit.
                  </span>
                </div>
              </div>
            </div>
            <div className="settings-featgroup">
              <span className="settings-featgroup-title">Cloudlog / Wavelog</span>
              <p className="settings-note">
                Auto-forward each logged QSO to your self-hosted <strong>Cloudlog</strong> or{' '}
                <strong>Wavelog</strong> logbook (HTTP). The API key is a per-instance token for your
                own server — enter it, your station-profile id, and turn on the toggle.
              </p>
              <div className="settings-grid">
                <label className="settings-field">
                  <span className="settings-label">Base URL</span>
                  <input
                    className="settings-input"
                    type="text"
                    value={form.cloudlogUrl ?? ''}
                    placeholder="https://log.example.com"
                    onChange={(e) => update('cloudlogUrl', e.target.value)}
                    autoComplete="off"
                    spellCheck={false}
                  />
                  <span className="settings-hint">Your Cloudlog/Wavelog site root. Leave blank to disable.</span>
                </label>

                <label className="settings-field">
                  <span className="settings-label">Station profile id</span>
                  <input
                    className="settings-input"
                    type="text"
                    inputMode="numeric"
                    value={form.cloudlogStationId ?? ''}
                    placeholder="1"
                    onChange={(e) => update('cloudlogStationId', e.target.value)}
                    autoComplete="off"
                  />
                  <span className="settings-hint">The station-location profile to log against (Cloudlog ▸ Station Locations).</span>
                </label>

                <label className="settings-field">
                  <span className="settings-label">API key</span>
                  <div className="settings-input-row">
                    <input
                      className="settings-input"
                      type="password"
                      value={cloudlogKey}
                      placeholder="your instance API key"
                      onChange={(e) => setCloudlogKeyField(e.target.value)}
                      autoComplete="off"
                      spellCheck={false}
                    />
                    <button
                      type="button"
                      className="settings-refresh"
                      onClick={onSaveCloudlogKey}
                      disabled={!cloudlogKey}
                    >
                      Set
                    </button>
                    <button
                      type="button"
                      className="settings-refresh"
                      onClick={onForgetCloudlogKey}
                      title="Remove the stored Cloudlog key from the system keychain"
                    >
                      Forget
                    </button>
                  </div>
                  <span className="settings-hint">
                    Cloudlog ▸ Account ▸ API Keys — a key with read/write. Stored in the OS keychain,
                    never on disk.
                  </span>
                </label>

                <div className="settings-field">
                  <label className="settings-toggle">
                    <span className="settings-label">Auto-forward QSOs</span>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={form.cloudlogUpload ?? false}
                      className={`toggle${form.cloudlogUpload ? ' on' : ''}`}
                      onClick={() => updateBool('cloudlogUpload', !form.cloudlogUpload)}
                    >
                      <span className="toggle-knob" />
                    </button>
                  </label>
                  <span className="settings-hint">Push every logged QSO to the instance above as it's logged.</span>
                </div>
              </div>
            </div>
          </fieldset>
          </>
          )}
          {/* ---- Field Day ---- */}
          {tab === 'fieldday' && (
          <>
          <fieldset className="settings-section">
            <legend>Field Day Setup</legend>
            {(!form.fdClass.trim() || !form.fdSection.trim()) && (
              <p className="settings-note">
                <strong>Set your Class + Section to start operating.</strong> Field Day mode is on,
                but the station won&apos;t enter Field Day until both are filled in below.
              </p>
            )}
            <div className="settings-grid">
              <div className="settings-field">
                <span className="settings-label">Event</span>
                <div className="theme-switcher" role="group" aria-label="Field Day event">
                  {([
                    { value: 'arrlfd', label: 'ARRL Field Day' },
                    { value: 'wfd',    label: 'Winter Field Day' },
                  ] as { value: string; label: string }[]).map((ev) => (
                    <button
                      key={ev.value}
                      type="button"
                      className={`theme-chip${(form.fdEvent ?? 'arrlfd') === ev.value ? ' active' : ''}`}
                      aria-pressed={(form.fdEvent ?? 'arrlfd') === ev.value}
                      onClick={() => {
                        markDirty()
                        setForm((prev) => prev ? { ...prev, fdEvent: ev.value } : prev)
                      }}
                    >
                      {ev.label}
                    </button>
                  ))}
                </div>
                <span className="settings-hint">Which event you're operating in — affects scoring labels and export headers.</span>
              </div>

              <label className="settings-field">
                <span className="settings-label">
                  {(form.fdEvent ?? 'arrlfd') === 'wfd' ? 'WFD Category' : 'FD Class'}
                </span>
                <input
                  className="settings-input mono"
                  type="text"
                  value={form.fdClass}
                  placeholder={(form.fdEvent ?? 'arrlfd') === 'wfd' ? '2O' : '1D'}
                  onChange={(e) => update('fdClass', e.target.value.toUpperCase())}
                  autoComplete="off"
                  spellCheck={false}
                />
                <span className="settings-hint">
                  {(form.fdEvent ?? 'arrlfd') === 'wfd'
                    ? 'Transmitters + location: H=Home, I=Indoor, M=Mobile, O=Outdoor (e.g. 2O = 2 transmitters, outdoor).'
                    : 'Number of transmitters + class letter: A=club/group portable, B=1–2 person portable, C=mobile, D=home (mains power), E=home (emergency power), F=EOC. E.g. 3A = 3 transmitters, club portable.'}
                </span>
              </label>

              <label className="settings-field">
                <span className="settings-label">ARRL Section</span>
                <input
                  className={`settings-input mono${fdSectionInvalid ? ' invalid' : ''}`}
                  type="text"
                  value={form.fdSection}
                  placeholder="WI"
                  list="fd-section-list"
                  aria-invalid={fdSectionInvalid}
                  onChange={(e) => update('fdSection', e.target.value.toUpperCase())}
                  autoComplete="off"
                  spellCheck={false}
                />
                <datalist id="fd-section-list">
                  {FD_SECTION_OPTIONS.map((s) => (
                    <option key={s.code} value={s.code}>{`${s.name} · ${s.division}`}</option>
                  ))}
                </datalist>
                {fdSectionInvalid && (
                  <span className="fd-section-warn" role="alert">
                    &ldquo;{form.fdSection}&rdquo; isn&apos;t a known ARRL/RAC section — pick one from the list.
                  </span>
                )}
                <span className="settings-hint">
                  Your ARRL / RAC section (e.g. WI, ENY, ONN). Start typing the code or a state name and pick from
                  the list — validated against all {FD_SECTION_OPTIONS.length} sections. Required for the Cabrillo log.
                </span>
              </label>

              <div className="settings-field">
                <span className="settings-label">Power multiplier</span>
                <div className="theme-switcher" role="group" aria-label="Field Day power multiplier">
                  {([
                    { value: 5, label: '×5 QRP / battery', hint: 'Runs entirely on battery or other natural power, ≤5W output' },
                    { value: 2, label: '×2 ≤100W',         hint: '100W or less from any power source' },
                    { value: 1, label: '×1 >100W',         hint: 'Over 100W — commercial/generator power' },
                  ] as { value: number; label: string; hint: string }[]).map((p) => (
                    <button
                      key={p.value}
                      type="button"
                      className={`theme-chip${(form.fdPowerMult ?? 1) === p.value ? ' active' : ''}`}
                      aria-pressed={(form.fdPowerMult ?? 1) === p.value}
                      title={p.hint}
                      onClick={() => {
                        markDirty()
                        setForm((prev) => prev ? { ...prev, fdPowerMult: p.value } : prev)
                      }}
                    >
                      {p.label}
                    </button>
                  ))}
                </div>
                <span className="settings-hint">
                  Multiplies your QSO points. QRP/battery = ×5 (ARRL bonus for going off-grid). Choose before the event.
                </span>
              </div>
            </div>
          </fieldset>

          <fieldset className="settings-section">
            <legend>N3FJP Integration (club master log)</legend>
            <p className="settings-note">
              Each FD contact lands in the club's{' '}
              <strong>N3FJP Field Day Contest Log</strong> the moment you log it — so the whole
              club's score updates in real time. Run N3FJP on the master computer; point Nexus at
              its IP + port (default 1100).
            </p>
            <div className="settings-grid">
              <label className="settings-field">
                <span className="settings-label">N3FJP host</span>
                <input
                  className="settings-input"
                  type="text"
                  value={form.n3fjpHost ?? ''}
                  placeholder="192.168.1.10 (empty = off)"
                  onChange={(e) => update('n3fjpHost', e.target.value)}
                  autoComplete="off"
                  spellCheck={false}
                />
                <span className="settings-hint">IP or hostname of the master log computer. Leave blank to disable.</span>
              </label>

              <label className="settings-field">
                <span className="settings-label">N3FJP port</span>
                <input
                  className="settings-input"
                  type="number"
                  inputMode="numeric"
                  value={form.n3fjpPort ?? 1100}
                  placeholder="1100"
                  onChange={(e) => {
                    markDirty()
                    setForm((prev) => prev ? { ...prev, n3fjpPort: Number(e.target.value) || 1100 } : prev)
                  }}
                  autoComplete="off"
                />
                <span className="settings-hint">N3FJP's API TCP port (default 1100).</span>
              </label>

              <div className="settings-field">
                <label className="settings-toggle">
                  <span className="settings-label">Use ENTER for Field Day scoring</span>
                  <button
                    type="button"
                    role="switch"
                    aria-checked={form.n3fjpUseEnter ?? true}
                    className={`toggle${(form.n3fjpUseEnter ?? true) ? ' on' : ''}`}
                    onClick={() => updateBool('n3fjpUseEnter', !(form.n3fjpUseEnter ?? true))}
                  >
                    <span className="toggle-knob" />
                  </button>
                </label>
                <span className="settings-hint">
                  Log each FD contact with N3FJP's <strong>ENTER</strong> sequence, which scores the
                  contest — the correct path. Turn off to fall back to a plain <code>ADDDIRECT</code>{' '}
                  insert (may not score). On by default.
                </span>
              </div>

              <div className="settings-field">
                <label className="settings-toggle">
                  <span className="settings-label">Report my band to N3FJP</span>
                  <button
                    type="button"
                    role="switch"
                    aria-checked={form.n3fjpReportBand ?? false}
                    className={`toggle${form.n3fjpReportBand ? ' on' : ''}`}
                    onClick={() => updateBool('n3fjpReportBand', !form.n3fjpReportBand)}
                  >
                    <span className="toggle-knob" />
                  </button>
                </label>
                <span className="settings-hint">
                  Tell N3FJP which band you're on (no CAT needed), so the club's Network Status
                  Display band board shows this position. Off by default.
                </span>
              </div>

              <div className="settings-field">
                <label className="settings-toggle">
                  <span className="settings-label">Forward every QSO</span>
                  <button
                    type="button"
                    role="switch"
                    aria-checked={form.n3fjpUpload ?? false}
                    className={`toggle${form.n3fjpUpload ? ' on' : ''}`}
                    onClick={() => updateBool('n3fjpUpload', !form.n3fjpUpload)}
                  >
                    <span className="toggle-knob" />
                  </button>
                </label>
                <span className="settings-hint">
                  Also push <strong>every</strong> logged QSO (not just Field Day) to N3FJP ACLog on
                  the host above — everyday general logging. N3FJP dedupes, so it's safe to run
                  alongside the Field-Day push.
                </span>
              </div>

              <div className="settings-field">
                <span className="settings-label">Connection test</span>
                <div className="settings-input-row">
                  <button
                    type="button"
                    className="settings-refresh"
                    onClick={runN3fjpTest}
                    disabled={n3fjpTest.state === 'testing' || !form.n3fjpHost?.trim()}
                    title="Save settings, then test the N3FJP TCP connection"
                  >
                    {n3fjpTest.state === 'testing' ? 'Testing…' : 'Test N3FJP'}
                  </button>
                </div>
                {n3fjpTest.state !== 'idle' && n3fjpTest.state !== 'testing' && (
                  <span className={`cat-result ${n3fjpTest.state}`} role="status">
                    {n3fjpTest.state === 'ok' ? '✓ ' : '✗ '}{n3fjpTest.msg}
                  </span>
                )}
                <span className="settings-hint">Run this at the club site before the event starts to confirm the API link works.</span>
              </div>
            </div>
          </fieldset>

          <fieldset className="settings-section">
            <legend>N1MM+ Integration</legend>
            <div className="settings-grid">
              <label className="settings-field">
                <span className="settings-label">N1MM contact broadcast address</span>
                <input
                  className="settings-input"
                  type="text"
                  value={form.n1mmAddr ?? ''}
                  placeholder="127.0.0.1:12060 (empty = off)"
                  onChange={(e) => update('n1mmAddr', e.target.value)}
                  autoComplete="off"
                  spellCheck={false}
                />
                <span className="settings-hint">
                  N1MM+ contact broadcast target (host:port, UDP). Nexus sends an N1MM-compatible
                  contact UDP packet for each FD QSO, so N1MM can display the contact on the network.
                  Leave blank to disable.
                </span>
              </label>
            </div>
          </fieldset>
          </>
          )}
        </div>

        <div className="settings-actions">
          {error && <span className="settings-error" role="alert">{error}</span>}
          {status === 'saved' && !error && (
            <span className="settings-ok" role="status">Saved</span>
          )}
          <button
            type="submit"
            className="settings-save"
            disabled={status === 'saving' || !form.mycall.trim()}
          >
            {status === 'saving' ? 'Saving…' : 'Save'}
          </button>
        </div>
      </form>
    </section>
  )
}
