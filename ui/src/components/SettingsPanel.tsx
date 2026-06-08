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
} from '../api'
import { pushToast, withErrorToast } from '../toast'
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
  { key: 'fdClass', label: 'Field Day Class', type: 'text', placeholder: '1D' },
  { key: 'fdSection', label: 'Field Day Section', type: 'text', placeholder: 'WI' },
]

const PTT_METHODS: { value: string; label: string }[] = [
  { value: 'cat', label: 'CAT (via rigctld)' },
  { value: 'rts', label: 'Serial RTS' },
  { value: 'dtr', label: 'Serial DTR' },
  { value: 'vox', label: 'VOX (no keying)' },
]

const NUMERIC_KEYS: FieldKey[] = ['dialMhz', 'baud', 'rigctldPort', 'rigModel', 'txWatchdogMin', 'catBrokerPort']

/** Settings is split into tabbed sections: only the active one renders, so a
 * keystroke re-renders ~one section's worth of inputs instead of the whole panel
 * (fixes typing lag) — and it tames the single-giant-scroll wall. */
type SettingsTab =
  | 'station'
  | 'rig'
  | 'audio'
  | 'operating'
  | 'alerts'
  | 'connections'
  | 'confirmations'
  | 'features'
  | 'workspace'

const SETTINGS_TABS: { id: SettingsTab; label: string }[] = [
  { id: 'station', label: 'Station' },
  { id: 'rig', label: 'Rig / CAT' },
  { id: 'audio', label: 'Audio' },
  { id: 'operating', label: 'Operating' },
  { id: 'alerts', label: 'Alerts' },
  { id: 'connections', label: 'Connections' },
  { id: 'confirmations', label: 'Confirmations' },
  { id: 'features', label: 'Features' },
  { id: 'workspace', label: 'Workspace' },
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
  const [bandPlan, setBandPlan] = useState<BandChannel[]>([])
  const [audio, setAudio] = useState<AudioDevices>({ input: [], output: [] })
  const [portsLoading, setPortsLoading] = useState(false)
  const [audioLoading, setAudioLoading] = useState(false)
  const [detected, setDetected] = useState<DetectedRig[]>([])
  const [detecting, setDetecting] = useState(false)
  const [catTesting, setCatTesting] = useState(false)
  const [catResult, setCatResult] = useState<CatTestResult | null>(null)
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
      pushToast('eQSL password saved to the system keychain', 'success')
    }
  }

  const onForgetEqslPassword = async () => {
    const ok = await withErrorToast(async () => {
      await clearEqslPassword()
      return true
    }, 'Could not clear the eQSL password')
    if (ok) {
      setEqslPw('')
      pushToast('eQSL password cleared from the keychain', 'success')
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
      pushToast('QRZ Logbook key saved to the system keychain', 'success')
    }
  }

  const onForgetQrzLogbookKey = async () => {
    const ok = await withErrorToast(async () => {
      await clearQrzLogbookKey()
      return true
    }, 'Could not clear the QRZ Logbook key')
    if (ok) {
      setQrzKey('')
      pushToast('QRZ Logbook key cleared from the keychain', 'success')
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
      pushToast('ClubLog app-password saved to the system keychain', 'success')
    }
  }

  const onForgetClublogPassword = async () => {
    const ok = await withErrorToast(async () => {
      await clearClublogPassword()
      return true
    }, 'Could not clear the ClubLog password')
    if (ok) {
      setClublogPw('')
      pushToast('ClubLog password cleared from the keychain', 'success')
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

  return (
    <section className="panel settings-panel">
      <div className="panel-header">
        <h2>Settings</h2>
        <span className="settings-sub">operator, rig &amp; network</span>
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
                </div>
                <span className="settings-hint">COM / tty device for rig control.</span>
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

              <div className="settings-field">
                <label className="settings-toggle">
                  <span className="settings-label">Let Nexus set the rig's mode</span>
                  <button
                    type="button"
                    role="switch"
                    aria-checked={form.setRigMode}
                    className={`toggle${form.setRigMode ? ' on' : ''}`}
                    onClick={() => updateBool('setRigMode', !form.setRigMode)}
                  >
                    <span className="toggle-knob" />
                  </button>
                </label>
                <span className="settings-hint">
                  Off (recommended): Nexus OBEYS whatever mode your radio is in (e.g. DATA-U) and
                  never changes it — maximum compatibility. On: Nexus forces the rig's DATA submode
                  (Yaesu DATA-U / Icom USB-D / Kenwood DATA) for digital.
                </span>
              </div>

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
                <span className="settings-label">Band (broadcast)</span>
                <input
                  className="settings-input"
                  type="text"
                  value={form.macros.band.join(', ')}
                  onChange={(e) => updateMacros('band', e.target.value)}
                  autoComplete="off"
                  spellCheck={false}
                />
                <span className="settings-hint">Chips for open broadcasts. (Field Day exchange stays automatic.)</span>
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
                  Surface "new ones" from the Reverse Beacon Network in Propagation → Needs heard now.
                  Takes effect on restart.
                </span>
              </div>

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
            </div>
          </fieldset>
          )}

          {/* ---- Confirmations (LoTW / eQSL / QRZ / ClubLog accounts) ---- */}
          {tab === 'confirmations' && (
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
                <span className="settings-label">ClubLog API key</span>
                <input
                  className="settings-input"
                  type="text"
                  value={form.clublogApiKey}
                  placeholder="get a free key at clublog.org/requestapikey.php"
                  onChange={(e) => update('clublogApiKey', e.target.value)}
                  autoComplete="off"
                  spellCheck={false}
                />
                <span className="settings-hint">
                  Nexus ships no ClubLog key (open-source — ClubLog auto-revokes published keys); request a free
                  one and paste it here.
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
