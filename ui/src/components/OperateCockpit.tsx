import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import type {
  AppSnapshot,
  BandChannel,
  ModeRequest,
  NeedAlert,
  NeedTag,
  Settings,
  SourceKind,
  Tier,
} from '../types'
import { bandLabelForMhz } from '../band'
import {
  clampOffsetHz,
  cqDirFromText,
  genStdMessages,
  snrForCall,
  stdMessageList,
  toggleIgnored,
} from '../txMessages'
import { openPanelWindow, getSettings, notifyErase, setSettings } from '../api'
import { pointRotatorAtCall, redecode, startCq, startQsoRecording, stopQsoRecording } from '../api'
import { setSkipTx1 as setSkipTx1Cmd } from '../api'
import { pushToast } from '../toast'
import { RotorStrip } from './RotorStrip'
import { Waterfall } from './Waterfall'
import { Splitter } from './Splitter'
import { buildHighlightMap, OperateDecodes } from './OperateDecodes'
import { OperateQsoStrip } from './OperateQsoStrip'
import { SpotDialog } from './SpotDialog'
import { OperateRoster } from './OperateRoster'
import { TxPanel } from './TxPanel'
import { CockpitHeader } from './CockpitHeader'
import { MemoryStrip } from './MemoryStrip'
import type { Memory } from '../features/memories'
import { FrequencyControl } from './FrequencyControl'
import { TuningStrip } from './TuningStrip'

interface Props {
  /** Configured companion UDP listen address (Settings) — shown instead of a
   * hardcoded :2237 so a moved WSJT-X port reads truthfully. */
  companionAddr?: string
  snap: AppSnapshot
  theme: string
  /** Active mode/tier (authoritative from the snapshot's link). */
  tier: Tier
  onTierChange: (t: Tier) => void
  /** Switch the RX signal source (native engine vs WSJT-X companion over UDP). */
  onSourceChange: (k: SourceKind) => void
  /** Click-to-tune on the waterfall: left=TX, right=RX, both buttons=TX+RX. */
  onTune: (freqHz: number, target: 'tx' | 'rx' | 'both') => void
  /** Work / answer a decoded station (double-click a decode or roster row). */
  onCall: (call: string, grid?: string, message?: string, snr?: number, freq?: number) => void
  /** Set the TX audio drive level (0.0–1.0) — the Pwr slider. */
  onSetTxLevel: (level: number) => void
  /** Band plan (channel select) shown in the header — FT8/FT4 freq+band moved
   * out of the global TopBar into the cockpit header. */
  bandPlan: BandChannel[]
  /** Commit a dial/band/mode change (the app's setFrequency handler). */
  onSetFrequency: (dialMhz: number, band: string, mode: string) => void
  /** Switch the QSO sequencer role (Call CQ / Monitor). */
  onSetMode: (mode: ModeRequest) => void
  /** Set the transmit period (Tx 1st/even vs Tx 2nd/odd). */
  onSetTxEven: (even: boolean) => void
  onSetTxCycleAuto: (auto: boolean) => void
  /** Re-arm the current QSO message. */
  onResend: () => void
  /** Send in-QSO free text (Tx5). */
  onFreetext: (text: string) => void
  /** Log the active QSO now (inline button). */
  onLog: () => void
  /** WSJT-X Tx-slot click: force `text` as the next transmission to `call`. */
  onOverrideTx: (call: string, grid: string | null, text: string) => void
  /** Halt TX immediately (the Esc key — same api as the Stop TX button). */
  onHaltTx: () => void
  /** TX-control cluster consolidated into the QSO strip (beside CQ/S&P). */
  onSetTxEnabled?: (on: boolean) => void
  onSetTune?: (on: boolean) => void
  onSetHoldTxFreq?: (on: boolean) => void
  /** Apply a fresh snapshot returned by a cockpit-local api call. */
  onSnap?: (s: AppSnapshot) => void
  /** Roger the final with RRR instead of RR73 (Settings preferRrr). */
  preferRrr?: boolean
  /** Bumps when a QSO was logged with "Clear DX call after logging" on — the
   * panel's DX fields wipe (stock WSJT-X option). */
  dxClearTick?: number
  /** Operator QSO macros — the Tx5 free-text datalist suggestions. */
  qsoMacros?: string[]
  /** The compact Call Roster (a wired StationList) shown in the Classic side column. */
  roster: ReactNode
  /** Award-need tier per call — drives the Roster layout's Need column + sort. */
  needByCall: Map<string, NeedTag>
  /** Full NeedAlerts per call — drives the band-activity decode feed's need icons +
   * row colour. Forwarded to every OperateDecodes instance. */
  needAlertsByCall?: Map<string, NeedAlert[]>
  /** Currently selected/open station (highlighted in the Roster layout). */
  selectedCall: string | null
  /** Select (open) a station from the Roster layout (single click). */
  onSelect: (call: string) => void
  /** Layout: 'classic' (WSJT-X — Band Activity dominant + compact roster aside) or
   * 'roster' (GridTracker — the full sortable Call Roster dominant). */
  layoutMode: 'classic' | 'roster'
  onLayoutMode: (m: 'classic' | 'roster') => void
  /** Open Operate in its own window (omit when already standalone). */
  onPopOut?: () => void
  /** Recall a saved memory (App applies settings + retune + cockpit switch).
   * Absent when the Memories feature is disabled — the MEM strip then hides. */
  onRecallMemory?: (m: Memory) => void
  /** Open the Memories section (manage/groups/import). */
  onOpenMemories?: () => void
  /** True when the cockpit is the active view. The cockpit stays MOUNTED across
   * navigation (so Band Activity keeps accumulating in the background); this flag
   * pauses the waterfall's render loop while it's hidden. */
  active?: boolean
}

/** Mode chips, in the order the cockpit presents them (popular modes first). */
// The DX-area cockpit operates the structured WSJT-X modes only. FT1/DX1 live in
// the MSG (Chat) area — no mixed tier picker.
const MODES: { tier: Tier; label: string; slot: string; title: string }[] = [
  { tier: 'FT8', label: 'FT8', slot: '15s', title: 'Standard WSJT-X FT8 — 15 s T/R' },
  { tier: 'FT4', label: 'FT4', slot: '7.5s', title: 'Standard WSJT-X FT4 — 7.5 s T/R' },
]

/** DXpedition special-op chip definitions. */
const SPECIAL_OPS: {
  value: NonNullable<Settings['specialOp']>
  label: string
  title: string
}[] = [
  { value: 'none', label: 'Off', title: 'No DXpedition special mode' },
  {
    value: 'hound',
    label: 'Hound',
    title:
      'DXpedition hound: calls go out above 1000 Hz; your R+report auto-moves to the Fox\'s frequency',
  },
  // SuperFox (superhound) retired by operator decision — the QPC table file's
  // license bars vendoring the native decoder outside WSJT-X. A settings file
  // that still says 'superhound' loads fine and behaves as plain Hound.
]

const NO_MACROS: string[] = []

/**
 * The Operate cockpit — the nerve center's primary operating surface. The
 * waterfall is the centerpiece (not a detached rail); a prominent mode selector
 * drives the live native decoder (FT8/FT4/FT1/DX1); the Band Activity table
 * accumulates, freezes-on-hover, filters and sorts. Click the waterfall to tune
 * RX (or shift-click for TX); single-click a decode to select it into the Tx
 * panel; double-click to work the station (stock WSJT-X click model).
 */
export function OperateCockpit({
  snap,
  theme,
  tier,
  onTierChange,
  bandPlan,
  onSetFrequency,
  onSourceChange,
  onTune,
  onCall,
  onSetTxLevel,
  onSetMode,
  onSetTxEven,
  onSetTxCycleAuto,
  onResend,
  onFreetext,
  onLog,
  onOverrideTx,
  onHaltTx,
  onSetTxEnabled,
  onSetTune,
  onSetHoldTxFreq,
  onSnap,
  preferRrr = false,
  dxClearTick = 0,
  qsoMacros = NO_MACROS,
  roster,
  needByCall,
  needAlertsByCall,
  selectedCall,
  onSelect,
  layoutMode,
  onLayoutMode,
  onPopOut,
  active = true,
  companionAddr,
  onRecallMemory,
  onOpenMemories,
}: Props) {
  // Container the waterfall-height splitter measures + writes its CSS var on.
  const bodyRef = useRef<HTMLDivElement>(null)
  const source = snap.radio.source

  // Live next-slot countdown: the snapshot's nextSlotMs only updates each poll,
  // so anchor it to wall-clock on each new value and tick locally for a smooth
  // 1-second cadence (the WSJT-X period clock operators watch).
  const slotBase = useRef({ ms: snap.radio.nextSlotMs, at: Date.now() })
  useEffect(() => {
    slotBase.current = { ms: snap.radio.nextSlotMs, at: Date.now() }
  }, [snap.radio.nextSlotMs])
  const [, tick] = useState(0)
  useEffect(() => {
    const id = window.setInterval(() => tick((t) => (t + 1) % 1000), 250)
    return () => window.clearInterval(id)
  }, [])
  const nextSlotSec = Math.max(
    0,
    Math.ceil((slotBase.current.ms - (Date.now() - slotBase.current.at)) / 1000),
  )

  // --- QSO recording (audio bridge) — same toggle as the Phone cockpit; the
  // global TopBar ● REC badge is the persistent stop once recording is on.
  const [recBusy, setRecBusy] = useState(false)
  // Tuning step (Hz) for the header's TuningStrip nudge/step — shared with the
  // step selector, same control CW/Phone carry (dial-nudge + VFO/RIT/XIT).
  const [tuneStep, setTuneStep] = useState(100)
  // Skip Tx1 (WSJT-X parity) — session-only UI state; the backend flag is likewise not
  // persisted, so both reset to off each launch. The toggle pushes to the engine.
  const [skipTx1, setSkipTx1] = useState(false)
  const handleSkipTx1 = useCallback((v: boolean) => {
    setSkipTx1(v)
    setSkipTx1Cmd(v).catch(() => {})
  }, [])
  const recording = snap.radio.qsoRecording
  const toggleRecord = () => {
    if (recBusy) return
    setRecBusy(true)
    const fn = recording ? stopQsoRecording : startQsoRecording
    fn()
      .then((s) => onSnap?.(s))
      .catch(() => pushToast(`Could not ${recording ? 'stop' : 'start'} recording`, 'error'))
      .finally(() => setRecBusy(false))
  }

  // --- DXpedition special-op selector ---
  // Fetch once on mount (lightweight — just reads the one field). After the
  // operator picks a chip we patch specialOp + save, then update local state.
  const [specialOp, setSpecialOp] = useState<NonNullable<Settings['specialOp']>>('none')
  const specialOpLoaded = useRef(false)
  useEffect(() => {
    let alive = true
    getSettings()
      .then((s) => {
        if (alive) {
          setSpecialOp(s.specialOp ?? 'none')
          specialOpLoaded.current = true
        }
      })
      .catch(() => {})
    return () => { alive = false }
  }, [])

  const handleSpecialOp = (val: NonNullable<Settings['specialOp']>) => {
    if (val === specialOp) return
    setSpecialOp(val)
    // Patch the one field: fetch current saved settings, apply the change, save.
    getSettings()
      .then((s) => setSettings({ ...s, specialOp: val }))
      .then((freshSnap) => onSnap?.(freshSnap))
      .catch(() => {})
  }

  // --- JTAlert highlight map (built once per highlights array reference) ---
  const highlightMap = useMemo(
    () => buildHighlightMap(snap.highlights),
    [snap.highlights],
  )

  // --- WSJT-X Tx1–Tx6 message machine (the Classic layout's Tx panel) ---
  const [dxCall, setDxCall] = useState('')
  // Spot-to-cluster popup: open state + the call to pre-fill (from the toolbar button or a roster row).
  const [spotOpen, setSpotOpen] = useState(false)
  const [spotSeedCall, setSpotSeedCall] = useState('')
  const openSpot = (call: string) => {
    setSpotSeedCall(call)
    setSpotOpen(true)
  }
  const [dxGrid, setDxGrid] = useState('')
  // Tx5 free text: auto-tracks the generated baseline until the operator edits
  // it; Generate Std Msgs resets the edit and re-baselines (stock behavior).
  const [tx5, setTx5] = useState('')
  const tx5Edited = useRef(false)
  // Tx6 free text: same auto-track pattern as Tx5, but follows msgs.tx6
  // (the CQ message) until the operator edits it for a directed CQ.
  const [tx6, setTx6] = useState('')
  const tx6Edited = useRef(false)
  // Locally picked "next" row (0-based) until qso.txNow confirms one.
  const [localNext, setLocalNext] = useState<number | null>(null)
  // Session-only ignore set (Alt-double-click a decode/roster row).
  const [ignored, setIgnored] = useState<ReadonlySet<string>>(() => new Set())

  // Waterfall pop-out: when the waterfall is torn off into its own window, unmount the docked
  // copy so the decode lists + roster reclaim the space (that's the whole point of popping it
  // out). Synced across windows by a persisted flag + the `storage` event, so closing the
  // pop-out re-docks automatically; a manual "re-dock" placeholder is the always-there fallback.
  // The flag is app-global (one waterfall pop-out), so in the rare two-main-window case both
  // main windows share it — an acceptable limitation for a single-pop-out feature. A stale flag
  // from a crash-while-popped-out is cleared on the next main-window boot (see main.tsx).
  const [wfDetached, setWfDetached] = useState(
    () => localStorage.getItem('nexus.waterfall.detached') === '1',
  )
  useEffect(() => {
    const onStorage = (e: StorageEvent) => {
      if (e.key === 'nexus.waterfall.detached') setWfDetached(e.newValue === '1')
    }
    window.addEventListener('storage', onStorage)
    return () => window.removeEventListener('storage', onStorage)
  }, [])
  const popOutWaterfall = () => {
    localStorage.setItem('nexus.waterfall.detached', '1')
    setWfDetached(true)
    void openPanelWindow('waterfall')
  }
  const redockWaterfall = () => {
    localStorage.setItem('nexus.waterfall.detached', '0')
    setWfDetached(false)
  }

  // RPT = the DX's current heard SNR (case-insensitive), −10 when unheard.
  const dxSnr = snrForCall(snap.stations, dxCall)
  const msgs = useMemo(
    () =>
      genStdMessages({
        dxCall,
        myCall: snap.mycall,
        myGrid: snap.mygrid,
        snr: dxSnr,
        preferRrr,
      }),
    [dxCall, snap.mycall, snap.mygrid, dxSnr, preferRrr],
  )
  useEffect(() => {
    if (!tx5Edited.current) setTx5(msgs.tx5)
  }, [msgs.tx5])
  useEffect(() => {
    if (!tx6Edited.current) setTx6(msgs.tx6)
  }, [msgs.tx6])

  const handleTx5 = (v: string) => {
    tx5Edited.current = true
    setTx5(v)
  }
  const handleTx6 = (v: string) => {
    tx6Edited.current = true
    setTx6(v)
  }
  const handleGenerate = () => {
    tx5Edited.current = false
    setTx5(msgs.tx5)
    tx6Edited.current = false
    setTx6(msgs.tx6)
  }
  const clearDx = useCallback(() => {
    setDxCall('')
    setDxGrid('')
    tx5Edited.current = false
    setTx5('')
    tx6Edited.current = false
    setTx6('')
    setLocalNext(null)
  }, [])

  // Stock "Clear DX call and grid after logging": App bumps the tick when a
  // QSO is logged with the option on. Skip the mount tick.
  const dxClearSeen = useRef(dxClearTick)
  useEffect(() => {
    if (dxClearTick !== dxClearSeen.current) {
      dxClearSeen.current = dxClearTick
      clearDx()
    }
  }, [dxClearTick, clearDx])

  // A retarget/abandon makes a remembered Tx-slot pick meaningless — without
  // this the next-dot stayed lit on a stale row of the NEW station's panel.
  const lastDx = useRef<string | null>(null)
  useEffect(() => {
    const dx = snap.qso?.dxcall ?? null
    if (dx !== lastDx.current) {
      lastDx.current = dx
      setLocalNext(null)
    }
  }, [snap.qso?.dxcall])

  // The six panel rows (Tx5 + Tx6 = the live editable texts). The "next" dot
  // follows qso.txNow when it matches a row, else the operator's local pick.
  const rowTexts = stdMessageList({ ...msgs, tx5, tx6 })
  const txNow = snap.qso?.txNow ?? null
  const liveNext = txNow ? rowTexts.indexOf(txNow) : -1
  const nextIndex = liveNext >= 0 ? liveNext : localNext

  /** Fire Tx row n (1-based).
   * Tx6 = Call CQ — parse the editable Tx6 text for a directed token and call
   * startCq(dir | null) directly; apply the returned snapshot via onSnap.
   * Tx1–Tx5 force the row's text as the next transmission to the DX. */
  const doTx = (n: number) => {
    if (n === 6) {
      setLocalNext(5)
      const dir = cqDirFromText(tx6, snap.mycall)
      // dir === undefined → parse failed / malformed → fall back to plain CQ
      const resolved = dir === undefined ? null : dir
      startCq(resolved).then((s) => onSnap?.(s)).catch(() => {})
      return
    }
    const call = dxCall.trim().toUpperCase()
    const text = rowTexts[n - 1]?.trim()
    if (!call || !text) return
    setLocalNext(n - 1)
    onOverrideTx(call, dxGrid.trim().toUpperCase() || null, text)
  }

  /** Re-decode the last period (WSJT-X Decode / F6). */
  const handleRedecode = () => {
    redecode().then((s) => onSnap?.(s)).catch(() => {})
  }

  /** Single-click SELECT from a decode: populate DX Call/Grid only — no RF
   * action, no TX (stock WSJT-X). A decoded grid wins; otherwise a stale grid
   * from a previous DX is cleared. */
  const selectDecode = (call: string, grid?: string) => {
    const up = call.trim().toUpperCase()
    if (grid) setDxGrid(grid.toUpperCase())
    else if (up !== dxCall) setDxGrid('')
    setDxCall(up)
  }

  const handleToggleIgnore = (call: string) => setIgnored((prev) => toggleIgnored(prev, call))
  const handleSetRx = (hz: number) => onTune(hz, 'rx')

  // Cockpit keyboard (stock WSJT-X): Esc = halt TX, F4 = clear DX, F6 = re-decode,
  // Alt+1…6 = the Tx buttons. Window-level, active-view only, and never while
  // typing in an input/textarea. Handlers ride a ref so the listener binds once
  // per activation without re-subscribing on every keystroke of state.
  const keyRef = useRef({ doTx, clearDx, halt: onHaltTx, redecode: handleRedecode })
  keyRef.current = { doTx, clearDx, halt: onHaltTx, redecode: handleRedecode }
  useEffect(() => {
    if (!active) return
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null
      const tag = t?.tagName
      if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || t?.isContentEditable) return
      if (e.key === 'Escape') {
        e.preventDefault()
        keyRef.current.halt()
        return
      }
      if (e.key === 'F4') {
        e.preventDefault()
        keyRef.current.clearDx()
        return
      }
      if (e.key === 'F6') {
        e.preventDefault()
        keyRef.current.redecode()
        return
      }
      const m = e.altKey && !e.ctrlKey && !e.metaKey ? /^Digit([1-6])$/.exec(e.code) : null
      if (m) {
        e.preventDefault()
        keyRef.current.doTx(Number(m[1]))
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [active])

  // Click-model props shared by every decode pane (Band Activity + Rx Freq in
  // both layouts) so select/ignore behave identically everywhere.
  const decodeClickProps = {
    onSelectDecode: selectDecode,
    onSetRx: handleSetRx,
    selectedCall: dxCall || null,
    ignoredCalls: ignored,
    onToggleIgnore: handleToggleIgnore,
    highlights: highlightMap,
    clearTick: snap.clearTick ?? 0,
  }

  // Active special-op badge shown next to the TX indicator.
  const specialOpBadge =
    specialOp === 'hound' ? (
      <span
        className="cockpit-specialop-badge hound"
        title="DXpedition hound: calls go out above 1000 Hz; your R+report auto-moves to the Fox's frequency"
      >
        HOUND
      </span>
    ) : specialOp === 'superhound' ? (
      // Retired option still present in an old settings file: same discipline.
      <span
        className="cockpit-specialop-badge hound"
        title="DXpedition hound: calls go out above 1000 Hz; your R+report auto-moves to the Fox's frequency"
      >
        HOUND
      </span>
    ) : null

  // Commit a typed dial from the shared header readout — routes through the
  // app's setFrequency handler (same path the old global TopBar readout used);
  // rejects out-of-plan frequencies with a toast.
  const commitDial = (mhz: number) => {
    const band = bandLabelForMhz(mhz)
    if (!band) {
      pushToast(`${mhz.toFixed(4)} MHz is outside the band plan`, 'error', 3000)
      return
    }
    onSetFrequency(mhz, band, snap.radio.sideband || 'USB')
  }

  return (
    <main className="layout single operate-cockpit">
      <CockpitHeader
        snap={snap}
        onSnap={onSnap}
        modeIndicator={
          <div className="cockpit-modes" role="group" aria-label="Operating mode">
            {MODES.map((m) => (
              <button
                key={m.tier}
                type="button"
                className={`cockpit-mode${tier === m.tier ? ' active' : ''}`}
                aria-pressed={tier === m.tier}
                onClick={() => onTierChange(m.tier)}
                title={m.title}
              >
                <span className="cm-name">{m.label}</span>
                <span className="cm-slot">{m.slot}</span>
              </button>
            ))}
          </div>
        }
        bandControl={
          <FrequencyControl
            channels={bandPlan}
            dialMhz={snap.radio.dialMhz}
            band={snap.radio.band}
            mode={snap.radio.sideband}
            variant="compact"
            showReadout={false}
            showModeToggle={false}
            onSet={onSetFrequency}
          />
        }
        onCommitDial={commitDial}
        frequencyExtras={
          <TuningStrip
            snap={snap}
            onSnap={onSnap}
            step={tuneStep}
            onStep={setTuneStep}
            showReadout={false}
          />
        }
        power={{
          value: snap.radio.txLevel,
          unit: 'drive',
          onChange: onSetTxLevel,
          label: 'Pwr',
          title: "TX drive (Pwr) — trim down until your rig's ALC is just zero",
        }}
        txState={false}
      >
        {/* DXpedition special-op selector — compact 3-chip control, always visible
            in both classic and roster layouts. Edits settings.specialOp. */}
        <div className="cockpit-specialop" role="group" aria-label="DXpedition mode">
          <span className="cockpit-specialop-label">DXped:</span>
          {SPECIAL_OPS.map((op) => (
            <button
              key={op.value}
              type="button"
              className={`cockpit-specialop-chip${specialOp === op.value ? ' active' : ''}`}
              aria-pressed={specialOp === op.value}
              onClick={() => handleSpecialOp(op.value)}
              title={op.title}
            >
              {op.label}
            </button>
          ))}
        </div>

        <div className="cockpit-meta">
          <div
            className="cockpit-source"
            role="group"
            aria-label="Signal source"
            title={`Where decodes come from — ${snap.radio.sourceLabel || 'native engine'}. Native = Nexus decodes local audio; Companion = ride an upstream WSJT-X/JTDX/MSHV decode stream over UDP ${companionAddr || '127.0.0.1:2237'}.`}
          >
            <button
              type="button"
              className={`cs-opt${source === 'native' ? ' active' : ''}`}
              aria-pressed={source === 'native'}
              onClick={() => onSourceChange('native')}
              title="Native engine — Nexus decodes local audio"
            >
              ◉ Native
            </button>
            <button
              type="button"
              className={`cs-opt${source === 'companion' ? ' active' : ''}`}
              aria-pressed={source === 'companion'}
              onClick={() => onSourceChange('companion')}
              title={`Companion — ride an existing WSJT-X / JTDX / MSHV decode stream over UDP ${companionAddr || '127.0.0.1:2237'}`}
            >
              ⇄ Companion
            </button>
          </div>
          <span className="cockpit-source-label" title="Active decode source">
            {snap.radio.sourceLabel || 'Native'}
            {source === 'companion' && ` · listening ${companionAddr || '127.0.0.1:2237'}`}
          </span>
          {/* DF readouts: type an exact audio offset and commit on Enter/blur
              (clamped to the 200–4000 Hz passband) — WSJT-X's Rx/Tx Hz spinners. */}
          <div className="cockpit-offsets" role="group" aria-label="Audio offsets (Hz)">
            <DfField label="Rx" hz={snap.radio.rxOffsetHz} onCommit={(hz) => onTune(hz, 'rx')} />
            <DfField label="Tx" hz={snap.radio.txOffsetHz} onCommit={(hz) => onTune(hz, 'tx')} />
          </div>
          {/* Decode button — re-run the decoder over the last period's audio (F6). */}
          <button
            type="button"
            className="cockpit-decode-btn"
            onClick={handleRedecode}
            title="Re-decode the last period (F6)"
          >
            Decode
          </button>
          <button
            type="button"
            className={`ph-rec${recording ? ' on' : ''}`}
            onClick={toggleRecord}
            disabled={recBusy}
            title={
              recording
                ? 'Stop recording this QSO'
                : 'Record the received audio to a WAV in the recordings folder'
            }
          >
            {recording ? '■ Recording' : '● Record QSO'}
          </button>
          {snap.radio.splitTxMhz != null && (
            <span
              className="cockpit-cat ok"
              title={`Rig split active — TX ${snap.radio.splitTxMhz.toFixed(4)} MHz (pile-up). Any QSY returns to simplex.`}
            >
              SPLIT ▲
            </span>
          )}
          <div className="cockpit-layout-toggle" role="group" aria-label="Operate layout">
            <button
              type="button"
              className={`clt-opt${layoutMode === 'classic' ? ' active' : ''}`}
              aria-pressed={layoutMode === 'classic'}
              onClick={() => onLayoutMode('classic')}
              title="Classic — WSJT-X layout (Band Activity dominant)"
            >
              Classic
            </button>
            <button
              type="button"
              className={`clt-opt${layoutMode === 'roster' ? ' active' : ''}`}
              aria-pressed={layoutMode === 'roster'}
              onClick={() => onLayoutMode('roster')}
              title="Roster — GridTracker layout (Call Roster dominant)"
            >
              Roster
            </button>
          </div>
          {onRecallMemory && (
            <MemoryStrip
              dialMhz={snap.radio.dialMhz}
              mode={tier === 'FT4' ? 'FT4' : 'FT8'}
              onRecall={onRecallMemory}
              onManage={onOpenMemories}
            />
          )}
          <button
            type="button"
            className="cockpit-popout"
            onClick={() => openSpot(dxCall || selectedCall || '')}
            title="Spot a callsign to the DX cluster (opens a popup — call, frequency, comment)"
          >
            📢 Spot
          </button>
          {onPopOut && (
            <button
              type="button"
              className="cockpit-popout"
              onClick={onPopOut}
              title="Open Operate in its own window (for a second monitor)"
            >
              ⧉ Pop out
            </button>
          )}
        </div>
      </CockpitHeader>

      <div className={`cockpit-status ${snap.radio.transmitting ? 'tx' : 'rx'}`}>
        <span className="cs-state">
          {snap.radio.transmitting ? '▲ TRANSMITTING' : snap.radio.txEnabled ? '▼ Receiving' : '■ TX off'}
        </span>
        {snap.radio.transmitting && snap.qso?.txNow && (
          <span className="cs-msg mono">{snap.qso.txNow}</span>
        )}
        {/* Active DXpedition mode badge — prominent, next to the TX indicator */}
        {specialOpBadge}
        <span className="cs-spacer" />
        <RotorStrip
          active={active}
          targetCall={selectedCall}
          onPointAt={(call) =>
            pointRotatorAtCall(call)
              .then((bearing) => pushToast(`Rotator → ${call}: ${Math.round(bearing)}°`, 'info'))
              .catch((e) => pushToast(`Rotator: ${e instanceof Error ? e.message : e}`, 'error'))
          }
        />
        <button
          type="button"
          className={`cs-period${snap.radio.txCycleAuto ? ' is-auto' : ''}`}
          onClick={() => {
            // Cycle: Auto → lock 1st → lock 2nd → Auto.
            if (snap.radio.txCycleAuto) onSetTxEven(true)
            else if (snap.radio.txEven) onSetTxEven(false)
            else onSetTxCycleAuto(true)
          }}
          title="Transmit cycle — click to cycle Auto → Tx 1st → Tx 2nd. Auto picks the opposite cycle of the station you answer; the station you work must be on the OPPOSITE period."
        >
          {snap.radio.txCycleAuto
            ? `TX AUTO / ${snap.radio.txEven ? '1st' : '2nd'}`
            : snap.radio.txEven
              ? 'TX 1st / even'
              : 'TX 2nd / odd'}
        </button>
        <button
          type="button"
          className={`cs-skiptx1${skipTx1 ? ' on' : ''}`}
          aria-pressed={skipTx1}
          onClick={() => handleSkipTx1(!skipTx1)}
          title="Skip Tx1 — when you answer a CQ, open with your signal report (Tx2) instead of your grid (Tx1), saving a cycle. Standard callsigns only (a compound call still sends its grid). Resets each launch, like WSJT-X."
        >
          Skip Tx1
        </button>
        <span className="cs-next" title="Time to the next slot">
          next {nextSlotSec}s
        </span>
      </div>

      <div className="cockpit-body" ref={bodyRef}>
        {/* Waterfall: a short full-width strip (not a tall column) — the spectrum
            is a glance tool; the real estate goes to the decode lists + roster. */}
        {wfDetached ? (
          <button
            type="button"
            className="wf-redock"
            onClick={redockWaterfall}
            title="The waterfall is in its own window — click to bring it back here"
          >
            ⧉ Waterfall popped out — click to re-dock
          </button>
        ) : (
          <>
            <section className="cockpit-waterfall panel">
              <button
                type="button"
                className="wf-popout"
                onClick={popOutWaterfall}
                title="Pop the waterfall out into its own window (frees this space; drag to another monitor)"
              >
                ⧉
              </button>
              <Waterfall
                transmitting={snap.radio.transmitting}
                rxOffsetHz={snap.radio.rxOffsetHz}
                txOffsetHz={snap.radio.txOffsetHz}
                theme={theme}
                onTune={onTune}
                active={active}
              />
            </section>
            <Splitter
              axis="y"
              varName="--cockpit-wf-h"
              target={bodyRef}
              storageKey="nexus.split.operate.waterfall"
              minPx={88}
              maxPx={420}
              defaultPct={22}
              label="waterfall height"
            />
          </>
        )}

        {/* Prominent operating bar: Call CQ / S&P / Now-sending / Resend / Tx5. */}
        <OperateQsoStrip
          qso={snap.qso}
          radio={snap.radio}
          onSetTxEnabled={onSetTxEnabled}
          onSetTune={onSetTune}
          onHaltTx={onHaltTx}
          onSetHoldTxFreq={onSetHoldTxFreq}
          onSetMode={onSetMode}
          onCallCq={() => {
            // The labelled "Call CQ" is always a PLAIN run — it also clears a
            // sticky directed token so a leftover "CQ DX" can't surprise.
            void startCq(null).then((s) => onSnap?.(s)).catch(() => {})
          }}
          onResend={onResend}
          onFreetext={onFreetext}
          onLog={onLog}
        />

        <div className={`cockpit-lower ${layoutMode}`}>
          {layoutMode === 'roster' ? (
            <>
              {/* Roster layout (GridTracker-style): the full sortable Call Roster is
                  the centerpiece; Band Activity + Rx Frequency move to a side rail. */}
              <div className="cockpit-roster-main panel">
                <OperateRoster
                  stations={snap.stations}
                  myGrid={snap.mygrid}
                  currentSlot={snap.radio.slot}
                  needByCall={needByCall}
                  needAlertsByCall={needAlertsByCall}
                  selectedCall={selectedCall}
                  onSelect={onSelect}
                  onCall={onCall}
                  ignoredCalls={ignored}
                  onToggleIgnore={handleToggleIgnore}
                  // Open the reviewable Spot popup pre-filled with this station (posting to a public
                  // cluster deserves a glance before it goes out); the dialog seeds the dial + a mode
                  // comment itself.
                  onSpot={(call) => openSpot(call)}
                />
              </div>
              <aside className="cockpit-side">
                <div className="cockpit-decodes-side panel">
                  {/* The FULL decode window (filters + sort), not the compact
                      strip — roster mode = decode window + roster on one page
                      (operator request); only Rx Frequency stays compact. */}
                  <OperateDecodes
                    decodes={snap.recentDecodes}
                    slot={snap.radio.slot}
                    rxOffsetHz={snap.radio.rxOffsetHz}
                    band={snap.radio.band}
                    tier={tier}
                    harqRescues={snap.harqRescues}
                    onCall={onCall}
                    needAlertsByCall={needAlertsByCall}
                    {...decodeClickProps}
                    onErase={() => notifyErase(0)}
                    title="Band Activity"
                  />
                </div>
                <div className="cockpit-rxfreq panel">
                  <OperateDecodes
                    decodes={snap.recentDecodes}
                    slot={snap.radio.slot}
                    rxOffsetHz={snap.radio.rxOffsetHz}
                    band={snap.radio.band}
                    tier={tier}
                    harqRescues={snap.harqRescues}
                    onCall={onCall}
                    needAlertsByCall={needAlertsByCall}
                    {...decodeClickProps}
                    onErase={() => notifyErase(1)}
                    lockedFilter="rx"
                    compact
                    title={`Rx Frequency · ${Math.round(snap.radio.rxOffsetHz)} Hz`}
                  />
                </div>
              </aside>
            </>
          ) : (
            <>
              {/* Classic layout (WSJT-X two-pane): Band Activity takes the full
                  left column; the compact Tx1–Tx6 message machine, Rx Frequency,
                  and the Stations roster ride the side rail. */}
              <div className="cockpit-decodes panel">
                <OperateDecodes
                  decodes={snap.recentDecodes}
                  slot={snap.radio.slot}
                  rxOffsetHz={snap.radio.rxOffsetHz}
                  band={snap.radio.band}
                  tier={tier}
                  harqRescues={snap.harqRescues}
                  onCall={onCall}
                  needAlertsByCall={needAlertsByCall}
                  {...decodeClickProps}
                  onErase={() => notifyErase(0)}
                />
              </div>
              <aside className="cockpit-side">
                <TxPanel
                  compact
                  dxCall={dxCall}
                  dxGrid={dxGrid}
                  onDxCall={setDxCall}
                  onDxGrid={setDxGrid}
                  messages={msgs}
                  tx5={tx5}
                  onTx5={handleTx5}
                  tx6={tx6}
                  onTx6={handleTx6}
                  nextIndex={nextIndex}
                  onTx={doTx}
                  onGenerate={handleGenerate}
                  onClear={clearDx}
                  qsoMacros={qsoMacros}
                />
                <div className="cockpit-rxfreq panel">
                  <OperateDecodes
                    decodes={snap.recentDecodes}
                    slot={snap.radio.slot}
                    rxOffsetHz={snap.radio.rxOffsetHz}
                    band={snap.radio.band}
                    tier={tier}
                    harqRescues={snap.harqRescues}
                    onCall={onCall}
                    needAlertsByCall={needAlertsByCall}
                    {...decodeClickProps}
                    onErase={() => notifyErase(1)}
                    lockedFilter="rx"
                    compact
                    title={`Rx Frequency · ${Math.round(snap.radio.rxOffsetHz)} Hz`}
                  />
                </div>
                <div className="cockpit-roster panel">{roster}</div>
              </aside>
            </>
          )}
        </div>
      </div>
      <SpotDialog
        open={spotOpen}
        onClose={() => setSpotOpen(false)}
        initialCall={spotSeedCall}
        freqMhz={snap.radio.dialMhz}
        defaultComment={String(snap.link.tier).toUpperCase()}
      />
    </main>
  )
}

/**
 * A compact labeled DF (audio offset) entry: tracks the snapshot value while
 * idle, commits on Enter/blur (rounded + clamped 200–4000 Hz), and reverts on
 * garbage input. Enter just blurs — the single commit happens in onBlur.
 */
function DfField({
  label,
  hz,
  onCommit,
}: {
  label: string
  hz: number
  onCommit: (hz: number) => void
}) {
  const [text, setText] = useState(() => String(Math.round(hz)))
  const [editing, setEditing] = useState(false)
  const editingRef = useRef(editing)
  editingRef.current = editing
  // Track the live prop ONLY when it actually changes — keying the effect on
  // `editing` too made the blur's clamped value flash back to the stale prop
  // for the IPC round-trip (commit sets text + editing in the same render).
  useEffect(() => {
    if (!editingRef.current) setText(String(Math.round(hz)))
  }, [hz])
  const commit = () => {
    setEditing(false)
    const n = Number(text)
    if (text.trim() !== '' && Number.isFinite(n)) {
      const clamped = clampOffsetHz(n)
      setText(String(clamped))
      if (clamped !== Math.round(hz)) onCommit(clamped)
    } else {
      setText(String(Math.round(hz))) // revert garbage
    }
  }
  return (
    <label className="df-field" title={`${label} audio offset (Hz) — Enter/blur commits, clamped 200–4000`}>
      <span className="df-label">{label}</span>
      <input
        type="number"
        inputMode="numeric"
        min={200}
        max={4000}
        step={1}
        value={text}
        aria-label={`${label} offset in Hz`}
        onFocus={() => setEditing(true)}
        onChange={(e) => setText(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === 'Enter') {
            e.preventDefault()
            ;(e.target as HTMLInputElement).blur()
          }
        }}
      />
      <span className="df-unit">Hz</span>
    </label>
  )
}
