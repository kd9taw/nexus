import { useEffect, useRef, useState } from 'react'
import { Dialog } from './ui/Dialog'
import { PROFILE_LIST, PROFILES, type ProfileId } from '../features/profiles'
import type { FeatureId, View } from '../features/registry'
import type { AudioDevices, CatTestResult, DetectedRig, ImportStats, Settings } from '../types'
import { detectRigs, discoverFlex, getAudioDevices, importAdif } from '../api'
import { isValidGrid } from '../grid'
import { findDaxDevices, isDaxPaired } from '../features/dax'

/** What the wizard collects beyond the goal profiles: station identity + rig.
 * Only the fields the operator actually touched are set — App merges this into
 * settings with ONE apply_settings write (it's a heavyweight call). */
export interface WizardDraft {
  mycall?: string
  mygrid?: string
  rigConn?: string
  rigAddr?: string
  rigModel?: number
  rigModelName?: string
  serialPort?: string
  audioIn?: string
  audioOut?: string
}

interface Props {
  /** Current settings — prefills so the Settings ▸ re-open path edits in place. */
  settings: Settings | null
  /** Apply goal profile(s) + modes + license + the station/rig draft, then navigate. */
  onApply: (
    ids: ProfileId[],
    landing: View,
    modes: FeatureId[],
    license: string,
    draft: WizardDraft,
  ) => void
  /** Save the draft so far and probe CAT against it (Settings' test, wizard-reachable). */
  onTestCat: (draft: WizardDraft) => Promise<CatTestResult>
  /** Close without changing the current feature set (also ESC / backdrop). */
  onSkip: () => void
}

// Goal cards are the five goal profiles; "Everything" is its own one-click button.
const GOALS = PROFILE_LIST.filter((p) => p.id !== 'everything')

// Operating modes are SEPARATE from goals (you can chase DX on any mode). Digital is
// always on (the FT8/FT4 cockpit is the core spine); Phone/CW are opt-in sections.
const MODES: { id: FeatureId; label: string; blurb: string }[] = [
  { id: 'phone', label: 'Phone (SSB)', blurb: 'Voice — PTT, sideband, panadapter' },
  { id: 'cw', label: 'CW', blurb: 'Morse — keyboard + macros, any rig' },
]

// License class → sets the transmit-privilege lockout + the licensed-segment band dropdown.
// "Outside the US" = Open (no transmit limits). Single-select; defaults to Open so the
// lockout is opt-in (a US op declares their class to turn it on).
const LICENSE: { id: string; label: string; blurb: string }[] = [
  { id: 'technician', label: 'Technician', blurb: 'US — limited HF + full VHF/UHF' },
  { id: 'general', label: 'General', blurb: 'US — most HF privileges' },
  { id: 'extra', label: 'Amateur Extra', blurb: 'US — full privileges' },
  { id: 'open', label: 'Outside the US', blurb: 'No transmit limits' },
]

/** Strict Maidenhead check — a persisted locator must be a real one, not just
 * something the lenient distance parser happens to swallow. */
const gridOk = isValidGrid

/**
 * First-run setup wizard — four short, individually skippable steps:
 * 1. STATION (callsign + grid — the locator every feature computes from),
 * 2. RIG & AUDIO (auto-detect / Serial-vs-Network with Find-my-Flex + DAX / audio),
 * 3. LOG (optional ADIF import — seeds worked-before / needs / awards from your history),
 * 4. GOALS (the original goal-driven preset selector — never asks for
 *    self-rated experience).
 * Everything stays changeable later in Settings; ESC/backdrop = skip-all
 * (marks seen, keeps the current feature set). Prefilled from settings so the
 * Settings ▸ re-open path acts as an editor. Built on the Radix [`Dialog`].
 * See feature-modularity.md §4.6.
 */
export function SetupWizard({ settings, onApply, onTestCat, onSkip }: Props) {
  const [step, setStep] = useState(0) // 0 station · 1 rig · 2 log · 3 goals

  // --- Step 3: optional ADIF log import (seeds worked-before / needs / awards) ---
  const [importStats, setImportStats] = useState<ImportStats | null>(null)
  const [importError, setImportError] = useState<string | null>(null)
  const [importing, setImporting] = useState(false)
  const logFileRef = useRef<HTMLInputElement>(null)
  const onImportAdif = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const f = e.target.files?.[0]
    e.target.value = '' // let the same file be re-picked
    if (!f) return
    setImporting(true)
    setImportError(null)
    setImportStats(null)
    try {
      // Feedback is INLINE (below), not a toast — a toast paints dimmed + undismissable behind
      // this modal wizard. A 0/0 result (non-ADIF file) is surfaced as a warning, not a success.
      setImportStats(await importAdif(await f.text()))
    } catch (err) {
      setImportError(`Import failed: ${err instanceof Error ? err.message : String(err)}`)
    } finally {
      setImporting(false)
    }
  }

  // --- Step 1: station identity ---
  const [mycall, setMycall] = useState(() => settings?.mycall ?? '')
  const [mygrid, setMygrid] = useState(() => settings?.mygrid ?? '')

  // --- Step 2: rig & audio (loaded lazily when the step opens) ---
  const [rigConn, setRigConn] = useState(() => settings?.rigConn ?? 'serial')
  const [rigAddr, setRigAddr] = useState(() => settings?.rigAddr ?? '')
  const [rigModel, setRigModel] = useState<number | undefined>(undefined)
  const [rigModelName, setRigModelName] = useState<string | undefined>(undefined)
  const [serialPort, setSerialPort] = useState(() => settings?.serialPort ?? '')
  const [audioIn, setAudioIn] = useState(() => settings?.audioIn ?? '')
  const [audioOut, setAudioOut] = useState(() => settings?.audioOut ?? '')
  const [audio, setAudio] = useState<AudioDevices>({ input: [], output: [] })
  const [detected, setDetected] = useState<DetectedRig[] | null>(null)
  const [detectedFlex, setDetectedFlex] = useState<{ model: string; nickname: string; ip: string }[]>([])
  const [detecting, setDetecting] = useState(false)
  const [flexNote, setFlexNote] = useState<string | null>(null)
  const [catResult, setCatResult] = useState<CatTestResult | null>(null)
  const [catTesting, setCatTesting] = useState(false)
  useEffect(() => {
    if (step !== 1) return
    getAudioDevices()
      .then(setAudio)
      .catch(() => {})
  }, [step])

  // --- Step 3: goals / modes / license (the original screen) ---
  const [selected, setSelected] = useState<Set<ProfileId>>(new Set())
  const toggle = (id: ProfileId) =>
    setSelected((s) => {
      const n = new Set(s)
      if (n.has(id)) n.delete(id)
      else n.add(id)
      return n
    })
  const [modes, setModes] = useState<Set<FeatureId>>(new Set())
  const toggleMode = (id: FeatureId) =>
    setModes((s) => {
      const n = new Set(s)
      if (n.has(id)) n.delete(id)
      else n.add(id)
      return n
    })
  const [license, setLicense] = useState('open')

  /** Only fields the operator actually changed vs the prefill go in the draft —
   * an untouched wizard must not rewrite settings it never asked about. */
  const draft = (): WizardDraft => {
    const d: WizardDraft = {}
    const call = mycall.trim().toUpperCase()
    if (call !== (settings?.mycall ?? '')) d.mycall = call
    const grid = mygrid.trim()
    if (grid !== (settings?.mygrid ?? '') && (grid === '' || gridOk(grid))) d.mygrid = grid
    if (rigConn !== (settings?.rigConn ?? 'serial')) d.rigConn = rigConn
    if (rigAddr.trim() !== (settings?.rigAddr ?? '')) d.rigAddr = rigAddr.trim()
    if (rigModel != null) {
      d.rigModel = rigModel
      d.rigModelName = rigModelName ?? ''
    }
    if (serialPort !== (settings?.serialPort ?? '')) d.serialPort = serialPort
    if (audioIn !== (settings?.audioIn ?? '')) d.audioIn = audioIn
    if (audioOut !== (settings?.audioOut ?? '')) d.audioOut = audioOut
    return d
  }

  const ids = [...selected]
  const landing: View = ids.length === 1 ? PROFILES[ids[0]].landing : 'operate'
  const goLabel =
    ids.length === 0
      ? 'Choose a goal'
      : ids.length === 1
        ? `Set up ${PROFILES[ids[0]].label}`
        : `Set up ${ids.length} goals`

  const gridState = mygrid.trim() === '' ? 'empty' : gridOk(mygrid) ? 'ok' : 'bad'
  const dax = findDaxDevices(audio.input, audio.output)

  const runDetect = () => {
    setDetecting(true)
    // One scan, every radio kind: USB enumeration + Flex LAN discovery run
    // together; either probe may fail without killing the other's results.
    Promise.all([
      detectRigs().catch(() => [] as DetectedRig[]),
      discoverFlex().catch(() => []),
    ])
      .then(([rigs, flexes]) => {
        setDetected(rigs)
        setDetectedFlex(flexes)
      })
      .finally(() => setDetecting(false))
  }
  const applyDetected = (r: DetectedRig) => {
    if (r.suggestedModel != null) {
      setRigModel(r.suggestedModel)
      setRigModelName(r.suggestedModelName ?? '')
    }
    setSerialPort(r.portName)
    setRigConn('serial')
    if (r.suggestedAudio) {
      setAudioIn(r.suggestedAudio)
      setAudioOut(r.suggestedAudio)
    }
  }
  const applyDetectedFlex = (f: { model: string; nickname: string; ip: string }) => {
    // The WSJT-X-proven path: CAT rides the SmartSDR CAT app on THIS PC
    // (model 2036 @ SmartSDR CAT's default slice-A TCP port 5002), never the
    // radio's own :4992 — Hamlib's direct native backend is alpha and failed
    // on a real 6400M. Discovery proves the radio is reachable.
    setRigConn('network')
    setRigAddr('127.0.0.1:5002')
    setRigModel(2036)
    setRigModelName('FlexRadio FLEX-6xxx (SmartSDR CAT)')
    setFlexNote(
      `${f.model}${f.nickname ? ` "${f.nickname}"` : ''} at ${f.ip} — CAT set via SmartSDR CAT (slice A, port 5002; a second slice uses 60001). Test CAT below.`,
    )
  }
  const runCatTest = () => {
    setCatTesting(true)
    setCatResult(null)
    onTestCat(draft())
      .then(setCatResult)
      .catch((e) =>
        setCatResult({ ok: false, detail: e instanceof Error ? e.message : String(e) }),
      )
      .finally(() => setCatTesting(false))
  }

  const stepTitles = ['Your station', 'Your rig', 'Your log', 'Your goals']

  return (
    <Dialog
      open
      // ESC / backdrop / close → skip (keeps the current set, marks seen).
      onOpenChange={(o) => {
        if (!o) onSkip()
      }}
      title="Set up Nexus"
      hideTitle
    >
      <div className="wizard-dots" aria-label={`Step ${step + 1} of ${stepTitles.length}: ${stepTitles[step]}`}>
        {stepTitles.map((t, i) => (
          <button
            key={t}
            type="button"
            className={`wizard-dot${i === step ? ' cur' : ''}${i < step ? ' done' : ''}`}
            // Same gate as Next: a malformed grid can't be walked past via the dots.
            disabled={step === 0 && gridState === 'bad' && i !== 0}
            onClick={() => setStep(i)}
            title={t}
          >
            <span className="wizard-dot-n">{i + 1}</span> {t}
          </button>
        ))}
      </div>

      {step === 0 && (
        <>
          <h2 className="wizard-title">Who&rsquo;s on the air?</h2>
          <p className="wizard-sub">
            Your grid square is the anchor for everything location-based — satellite passes,
            propagation, the map, and DXpedition windows are all computed from it.
          </p>
          <div className="wizard-fields">
            <label className="wizard-field">
              <span>Callsign</span>
              <input
                type="text"
                value={mycall}
                placeholder="KD9TAW"
                autoComplete="off"
                spellCheck={false}
                onChange={(e) => setMycall(e.target.value.toUpperCase())}
              />
            </label>
            <label className="wizard-field">
              <span>Grid square</span>
              <input
                type="text"
                value={mygrid}
                placeholder="EN52"
                autoComplete="off"
                spellCheck={false}
                className={gridState === 'bad' ? 'bad' : ''}
                onChange={(e) => setMygrid(e.target.value)}
              />
              <span className={`wizard-field-hint${gridState === 'bad' ? ' bad' : ''}`}>
                {gridState === 'bad'
                  ? 'Not a Maidenhead locator — 4 or 6 characters, like EN52 or EN52xa.'
                  : 'Maidenhead locator (qrz.com shows yours). 4 characters is plenty.'}
              </span>
            </label>
          </div>
        </>
      )}

      {step === 1 && (
        <>
          <h2 className="wizard-title">How does the radio connect?</h2>
          <p className="wizard-sub">
            One detect finds everything — USB rigs and FlexRadios on the network.
            Skippable; Settings ▸ Rig Control has all of this later (including Test CAT).
          </p>
          <div className="wizard-detect">
            <button type="button" className="wizard-btn" disabled={detecting} onClick={runDetect}>
              {detecting ? 'Detecting…' : '🔍 Detect my radio'}
            </button>
            {detected != null && detected.length === 0 && detectedFlex.length === 0 && (
              <span className="wizard-field-hint">
                Nothing found — USB: plug in + power on; Flex: must be on this network.
                Or skip and set it up later.
              </span>
            )}
            {detectedFlex.map((f) => (
              <button
                key={f.ip}
                type="button"
                className={`wizard-detect-row${rigConn === 'network' && rigModel === 2036 ? ' sel' : ''}`}
                onClick={() => applyDetectedFlex(f)}
              >
                <b>
                  {f.model}
                  {f.nickname ? ` “${f.nickname}”` : ''}
                </b>{' '}
                on the network ({f.ip})
                <span className="wizard-field-hint"> · via SmartSDR CAT</span>
              </button>
            ))}
            {(detected ?? []).map((r) => (
              <button
                key={r.portName}
                type="button"
                className={`wizard-detect-row${serialPort === r.portName ? ' sel' : ''}`}
                onClick={() => applyDetected(r)}
              >
                <b>{r.suggestedModelName ?? r.product ?? 'Unknown radio'}</b> on {r.portName}
                <span className="wizard-field-hint"> · {r.chip}</span>
              </button>
            ))}
            {flexNote && <span className="wizard-field-hint">{flexNote}</span>}
            {rigConn === 'serial' && serialPort && (
              <span className="wizard-field-hint">
                Selected: {rigModelName ?? 'radio'} on {serialPort}
              </span>
            )}
          </div>
          <div className="wizard-rigconn">
            <button
              type="button"
              className={`wizard-mode${rigConn === 'serial' ? ' sel' : ''}`}
              aria-pressed={rigConn === 'serial'}
              onClick={() => setRigConn('serial')}
            >
              <span className="wizard-mode-label">USB / Serial</span>
              <span className="wizard-mode-blurb">Most rigs — one cable</span>
            </button>
            <button
              type="button"
              className={`wizard-mode${rigConn === 'network' ? ' sel' : ''}`}
              aria-pressed={rigConn === 'network'}
              onClick={() => setRigConn('network')}
            >
              <span className="wizard-mode-label">Network</span>
              <span className="wizard-mode-blurb">FlexRadio / remote rigctld</span>
            </button>
          </div>

          {rigConn === 'network' && (
            <div className="wizard-detect">
              <label className="wizard-field">
                <span>Address</span>
                <input
                  type="text"
                  value={rigAddr}
                  placeholder="127.0.0.1:5002"
                  autoComplete="off"
                  spellCheck={false}
                  onChange={(e) => setRigAddr(e.target.value)}
                />
              </label>
              <span className="wizard-field-hint">
                A found Flex configures the WSJT-X-proven path: CAT through the SmartSDR
                CAT app on this PC — its default TCP port 5002 drives slice A (per-slice
                ports: B=60001, C=60002) — and audio through DAX. Other network rigs:
                pick their model later in Settings ▸ Rig Control.
              </span>
              {dax && !isDaxPaired(audioIn, audioOut) && (
                <button
                  type="button"
                  className="wizard-btn"
                  onClick={() => {
                    setAudioIn(dax.input)
                    setAudioOut(dax.output)
                  }}
                  title="SmartSDR's DAX virtual audio devices were detected — pairs them as Nexus's audio in/out"
                >
                  ⚡ Pair DAX audio ({dax.input})
                </button>
              )}
            </div>
          )}

          <div className="wizard-fields">
            <label className="wizard-field">
              <span>Audio in</span>
              <select value={audioIn} onChange={(e) => setAudioIn(e.target.value)}>
                <option value="">System default</option>
                {/* Saved-but-unplugged device stays selectable (the Settings rule) —
                    a blank select that can only LOSE the saved routing is a trap. */}
                {[...new Set([...(audioIn ? [audioIn] : []), ...audio.input])].map((d) => (
                  <option key={d} value={d}>
                    {d}
                  </option>
                ))}
              </select>
            </label>
            <label className="wizard-field">
              <span>Audio out</span>
              <select value={audioOut} onChange={(e) => setAudioOut(e.target.value)}>
                <option value="">System default</option>
                {[...new Set([...(audioOut ? [audioOut] : []), ...audio.output])].map((d) => (
                  <option key={d} value={d}>
                    {d}
                  </option>
                ))}
              </select>
            </label>
          </div>

          <div className="wizard-detect">
            <button
              type="button"
              className="wizard-btn"
              disabled={catTesting}
              onClick={runCatTest}
              title="Saves what you've entered so far, then asks the radio for its frequency"
            >
              {catTesting ? 'Testing…' : '⚡ Test CAT'}
            </button>
            {catResult && (
              <span className={`wizard-field-hint${catResult.ok ? '' : ' bad'}`}>
                {catResult.ok ? '✓ ' : '✗ '}
                {catResult.detail}
              </span>
            )}
          </div>
        </>
      )}

      {step === 2 && (
        <>
          <h2 className="wizard-title">Bring in your existing log</h2>
          <p className="wizard-sub">
            Nexus works best when it knows your history. Importing your ADIF log is what powers{' '}
            <strong>worked-before</strong> flags, the <strong>Needed</strong> board (new DXCC / states
            / grids), and your <strong>awards</strong> progress — without it, the app starts blind and
            treats every station as new. This is optional and you can import anytime from the Logbook,
            but it's the single biggest thing that makes the app useful on day one.
          </p>
          <div className="wizard-log-import">
            <input
              ref={logFileRef}
              type="file"
              accept=".adi,.adif,text/plain"
              style={{ display: 'none' }}
              onChange={onImportAdif}
            />
            <button
              type="button"
              className="wizard-go"
              disabled={importing}
              onClick={() => logFileRef.current?.click()}
            >
              {importing ? 'Importing…' : importStats ? 'Import another ADIF file' : 'Import my ADIF log…'}
            </button>
            {importError && (
              <p className="wizard-log-error" role="alert">
                ⚠ {importError}
              </p>
            )}
            {importStats &&
              (importStats.added > 0 ? (
                <p className="wizard-log-result" role="status">
                  ✓ Imported <strong>{importStats.added}</strong>{' '}
                  QSO{importStats.added === 1 ? '' : 's'}
                  {importStats.skipped ? ` · ${importStats.skipped} already present` : ''}. Your
                  worked-before and Needed board are now seeded.
                </p>
              ) : importStats.skipped > 0 ? (
                <p className="wizard-log-result" role="status">
                  ✓ All {importStats.skipped} QSOs were already in your log — you're seeded.
                </p>
              ) : (
                <p className="wizard-log-error" role="status">
                  ⚠ No QSOs found in that file — is it a standard ADIF (.adi/.adif) export?
                </p>
              ))}
            <p className="wizard-license-sub">
              From WSJT-X, N1MM, Log4OM, HRD, QRZ, LoTW, ClubLog — any standard ADIF (.adi/.adif)
              export. Nothing leaves your computer; duplicates are detected and skipped.
            </p>
          </div>
        </>
      )}

      {step === 3 && (
        <>
          <h2 className="wizard-title">What do you mostly want to do?</h2>
          <p className="wizard-sub">
            Pick one or more — we’ll turn on the right features. You can change everything later in
            Settings → Features.
          </p>

          <div className="wizard-goals">
            {GOALS.map((p) => (
              <button
                key={p.id}
                type="button"
                className={`wizard-goal${selected.has(p.id) ? ' sel' : ''}`}
                aria-pressed={selected.has(p.id)}
                onClick={() => toggle(p.id)}
              >
                <span className="wizard-goal-label">{p.label}</span>
                <span className="wizard-goal-blurb">{p.blurb}</span>
              </button>
            ))}
          </div>

          <h3 className="wizard-modes-title">Which modes do you operate?</h3>
          <div className="wizard-modes">
            <button type="button" className="wizard-mode sel locked" aria-pressed disabled>
              <span className="wizard-mode-label">Digital (FT8/FT4)</span>
              <span className="wizard-mode-blurb">Always on — the waterfall cockpit</span>
            </button>
            {MODES.map((m) => (
              <button
                key={m.id}
                type="button"
                className={`wizard-mode${modes.has(m.id) ? ' sel' : ''}`}
                aria-pressed={modes.has(m.id)}
                onClick={() => toggleMode(m.id)}
              >
                <span className="wizard-mode-label">{m.label}</span>
                <span className="wizard-mode-blurb">{m.blurb}</span>
              </button>
            ))}
          </div>

          <h3 className="wizard-modes-title">What’s your license?</h3>
          <p className="wizard-license-sub">
            Sets your transmit privileges — the app parks the dial in your licensed band segments
            and won’t let you transmit outside them. Pick “Outside the US” for no limits.
          </p>
          <div className="wizard-modes">
            {LICENSE.map((l) => (
              <button
                key={l.id}
                type="button"
                className={`wizard-mode${license === l.id ? ' sel' : ''}`}
                aria-pressed={license === l.id}
                onClick={() => setLicense(l.id)}
              >
                <span className="wizard-mode-label">{l.label}</span>
                <span className="wizard-mode-blurb">{l.blurb}</span>
              </button>
            ))}
          </div>
        </>
      )}

      <div className="wizard-actions">
        {step === 3 ? (
          <button
            type="button"
            className="wizard-everything"
            onClick={() => onApply(['everything'], 'operate', [], license, draft())}
          >
            Turn everything on (expert)
          </button>
        ) : (
          <span />
        )}
        <div className="wizard-actions-right">
          {step > 0 && (
            <button type="button" className="wizard-skip" onClick={() => setStep(step - 1)}>
              ← Back
            </button>
          )}
          <button type="button" className="wizard-skip" onClick={onSkip}>
            I’ll set it up myself
          </button>
          {step < 3 ? (
            <button
              type="button"
              className="wizard-go"
              // A malformed grid must not ride into settings; empty is fine (skip).
              disabled={step === 0 && gridState === 'bad'}
              onClick={() => setStep(step + 1)}
            >
              Next →
            </button>
          ) : (
            <button
              type="button"
              className="wizard-go"
              disabled={ids.length === 0}
              onClick={() => onApply(ids, landing, [...modes], license, draft())}
            >
              {goLabel}
            </button>
          )}
        </div>
      </div>
    </Dialog>
  )
}
