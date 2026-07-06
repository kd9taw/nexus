import { useEffect, useState } from 'react'
import type { AudioDevices, BandChannel, CatTestResult, DetectedRig, RadioStatus, Settings } from '../types'
import {
  clearClublogPassword,
  clearEqslPassword,
  clearLotwPassword,
  clearQrzLogbookKey,
  clearQrzPassword,
  detectRigs,
  downloadEqslReport,
  downloadLotwReport,
  getAudioDevices,
  getBandPlan,
  getRigModels,
  getSerialPorts,
  getSettings,
  setClublogPassword,
  setEqslPassword,
  setLotwPassword,
  setQrzLogbookKey,
  setQrzPassword,
  setSettings,
  testCat,
  probeCatPorts,
  qrzTestConnection,
  n3fjpTestConnection,
} from '../api'
import { pushToast, withErrorToast } from '../toast'
import { loadProfiles, saveProfile, deleteProfile, type Profile } from '../profiles'
import { getConnectionLog, getCredentialsStatus } from '../api'
import { fetchLotwUsers, getLotwUsersStatus, type LotwUsersStatus } from '../api'
import type { ConnEvent, CredStatus } from '../types'
import { FrequencyControl } from './FrequencyControl'
import { LevelMeter } from './LevelMeter'
import type { Layout } from '../useLayout'
import type { Scale } from '../useScale'
import { SCALE_STEPS } from '../useScale'
import type { FeaturesApi } from '../useFeatures'
import { FEATURES, featureById, type FeatureCategory, type FeatureDef, type FeatureId } from '../features/registry'
import { PROFILE_LIST } from '../features/profiles'

interface Props {
  /** Called after a successful save so the shell can refresh its snapshot. */
  onSaved?: () => void
  /** Live radio status, so the Audio section can show the real RX meter. */
  radio?: RadioStatus
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
const FREQ_BANDS = ['160m', '80m', '60m', '40m', '30m', '20m', '17m', '15m', '12m', '10m', '6m', '2m', '70cm']
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
  { id: 'connections', label: 'Connections' },
  { id: 'confirmations', label: 'Confirmations' },
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
  const [serialPorts, setSerialPorts] = useState<string[]>([])
  const [profiles, setProfiles] = useState<Profile[]>(() => loadProfiles())
  const [selectedProfile, setSelectedProfile] = useState('')
  const [newProfileName, setNewProfileName] = useState('')
  const [bandPlan, setBandPlan] = useState<BandChannel[]>([])
  const [audio, setAudio] = useState<AudioDevices>({ input: [], output: [] })
  const [portsLoading, setPortsLoading] = useState(false)
  const [audioLoading, setAudioLoading] = useState(false)
  const [detected, setDetected] = useState<DetectedRig[]>([])
  const [detecting, setDetecting] = useState(false)
  const [catTesting, setCatTesting] = useState(false)
  const [catResult, setCatResult] = useState<CatTestResult | null>(null)
  // Connections visibility: stored-credential status + the rolling event log —
  // the answer to "I hit save and couldn't tell anything happened".
  const [creds, setCreds] = useState<CredStatus[]>([])
  // ARRL LoTW user-activity list (the decode/roster LoTW marks).
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
  const [clublogPw, setClublogPw] = useState('')
  const [tab, setTab] = useState<SettingsTab>('station')
  // In-progress MHz text for the override row being edited — committed only when
  // it parses as a positive number, so a half-typed "14." never corrupts the form.
  const [mhzDraft, setMhzDraft] = useState<{ idx: number; text: string } | null>(null)

  useEffect(() => {
    let mounted = true
    setStatus('loading')
    getSettings()
      .then((s) => {
        if (mounted) {
          setForm(s)
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

  const markDirty = () => {
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

  const selectRig = (modelNum: number) => {
    markDirty()
    const name = rigModels.find((m) => m[0] === modelNum)?.[1] ?? ''
    setForm((prev) => (prev ? { ...prev, rigModel: modelNum, rigModelName: name } : prev))
  }

  // Zero-config: scan connected USB radios.
  const onDetectRigs = async () => {
    setDetecting(true)
    const rigs = await withErrorToast(() => detectRigs(), 'Radio detection failed')
    setDetecting(false)
    if (rigs) {
      setDetected(rigs)
      if (rigs.length === 0) pushToast('No USB radios detected — plug one in, then Detect again.', 'info')
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
        const next = {
          ...form,
          serialPort: r.portName,
          baud: r.baud,
          rigModel: r.model,
          rigModelName: r.modelName,
          pttMethod: 'cat',
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
      setStatus('saved')
      onSaved?.()
    } catch {
      setStatus('idle')
      setError('Could not save settings.')
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

  return (
    <section className="panel settings-panel">
      <div className="panel-header">
        <h2>Settings</h2>
        <span className="settings-sub">operator, rig &amp; network</span>
        <span className="settings-build" title="This install's build stamp — confirm a fresh install actually took">
          build {__BUILD_ID__}
        </span>
      </div>

      <form className="settings-form" onSubmit={handleSubmit}>
        <div className="settings-tabs" role="tablist" aria-label="Settings sections">
          {SETTINGS_TABS.map((t) => (
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

            {/* Optional features, grouped by category. */}
            {FEATURE_CATEGORY_ORDER.map((cat) => {
              const inCat = FEATURES.filter((f) => f.category === cat && !f.core)
              if (inCat.length === 0) return null
              return (
                <div className="settings-featgroup" key={cat}>
                  <span className="settings-featgroup-title">{cat}</span>
                  <div className="settings-grid">{inCat.map(featureRow)}</div>
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
                {detected.length > 0 && (
                  <ul className="rig-detect-list">
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
                            <span className="rig-detect-meta">
                              Couldn't identify the model from USB — pick it below.
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
                  Scans connected USB radios and fills the model, port, and sound device below. Review, then Save.
                </span>
              </div>

              <label className="settings-field">
                <span className="settings-label">Rig Model</span>
                <select
                  className="settings-input"
                  value={String(form.rigModel)}
                  onChange={(e) => selectRig(Number(e.target.value))}
                >
                  <option value="0">— None —</option>
                  {rigModels.map(([num, name]) => (
                    <option key={num} value={String(num)}>
                      {name} ({num})
                    </option>
                  ))}
                </select>
                <span className="settings-hint">Hamlib rig model.</span>
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
                    placeholder="192.168.1.50:4992"
                    onChange={(e) => update('rigAddr', e.target.value)}
                    autoComplete="off"
                    spellCheck={false}
                  />
                  <span className="settings-hint">
                    host:port of the radio — a FlexRadio's IP on SmartSDR's port 4992, or a
                    remote rigctld. Pick the DAX audio devices under Audio.
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
                <input
                  className="settings-input"
                  type="number"
                  inputMode="numeric"
                  value={String(form.baud)}
                  placeholder="38400"
                  onChange={(e) => update('baud', e.target.value)}
                  autoComplete="off"
                />
                <span className="settings-hint">Serial baud rate.</span>
              </label>
                </>
              )}

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

              <label className="settings-field">
                <span className="settings-label">Rotator (rotctld)</span>
                <input
                  className="settings-input"
                  type="text"
                  value={form.rotatorHost}
                  placeholder="host:port — e.g. 127.0.0.1:4533"
                  onChange={(e) => update('rotatorHost', e.target.value)}
                  autoComplete="off"
                  spellCheck={false}
                />
                <span className="settings-hint">
                  Run rotctld; then click ↗ on a Needed row to point the antenna. Empty = off.
                </span>
              </label>

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
            </div>
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
          )}

          {/* ---- Operating ---- */}
          {tab === 'operating' && (
          <fieldset className="settings-section">
            <legend>Operating</legend>
            <div className="settings-grid">
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
          </fieldset>
          </>
          )}

          {/* ---- Network integrations ---- */}
          {tab === 'connections' && (
          <fieldset className="settings-section">
            <legend>Connections</legend>
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
                  Stored in the OS keychain, never on disk. <strong>Grid &amp; state require a QRZ XML
                  subscription</strong> — free accounts return only name/address/country.
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
                  Push each logged QSO to ClubLog in real time (needs the email, app-password, and API key above).
                </span>
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
                    : 'E.g. 1D (1 transmitter, EOC). Set before Field Day starts.'}
                </span>
              </label>

              <label className="settings-field">
                <span className="settings-label">ARRL Section</span>
                <input
                  className="settings-input mono"
                  type="text"
                  value={form.fdSection}
                  placeholder="WI"
                  onChange={(e) => update('fdSection', e.target.value.toUpperCase())}
                  autoComplete="off"
                  spellCheck={false}
                />
                <span className="settings-hint">Your ARRL / RAC section (e.g. WI, ENY, ONN). Required for the Cabrillo log.</span>
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
