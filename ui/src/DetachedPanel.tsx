// Standalone-window renderer: when the app is loaded at `?panel=<name>` (a torn-off
// window created by open_panel_window), render JUST that panel — chrome-less, with its
// own polling — against the same shared engine the main window uses. Multi-monitor
// tear-off: pop Connect / DXpeditions / the Operate cockpit / the Needed board onto
// separate displays so the operator stops toggling. Each detached window is its own
// independent client of the one shared Rust engine (snapshot at 300 ms; the Waterfall
// self-fetches its spectrum), and its action callbacks drive the same engine, so state
// stays consistent across every window.
import { useEffect, useMemo, useState } from 'react'
import type {
  AppSnapshot,
  BandChannel,
  Conversation as Conv,
  ModeRequest,
  NeedAlert,
  NeedTag,
  PropagationSnapshot,
  Settings,
  SourceKind,
  Tier,
} from './types'
import {
  getBandPlan,
  getNeedAlerts,
  getPropagation,
  getSettings,
  selectPeer,
  setFrequency,
  workSpot,
  subscribeSnapshot,
  callStation,
  setTier,
  setSource,
  setTxLevel,
  setMode,
  setTxEven,
  setTxCycleAuto,
  qsoResend,
  qsoFreetext,
  logCurrentQso,
  overrideNextTx,
  haltTx,
  setRxOffset,
  setTxOffset,
  pointRotatorAtCall,
} from './api'
import { NeededPanel } from './components/NeededPanel'
import { ConnectView } from './components/ConnectView'
import { DxpeditionsView } from './components/DxpeditionsView'
import { OperateCockpit } from './components/OperateCockpit'
import { StationList } from './components/StationList'
import { visibleNeeds, modeClassOf, workTarget } from './features/needs'
import { readEnabledModes } from './useFeatures'
import { useTheme } from './useTheme'
import { useScale } from './useScale'
import { useViewport } from './useViewport'
import { useDensity } from './useDensity'
import { useMotion } from './useMotion'

type SpotTarget = { call: string; band: string; mode: string | null; freqMhz: number | null }
type OperateLayout = 'classic' | 'roster'

function loadOperateLayout(): OperateLayout {
  try {
    return localStorage.getItem('nexus.operate.layout') === 'roster' ? 'roster' : 'classic'
  } catch {
    return 'classic'
  }
}

export function DetachedPanel({ panel }: { panel: string }) {
  const [theme] = useTheme()
  // A torn-off window is its OWN document — it must publish the same layout/responsive
  // state the main app does, or the CSS falls back to the broken narrow/stacked layout
  // (vertical rails go horizontal, the map collapses to zero height). Mirror App.tsx.
  const [scale] = useScale()
  useViewport(scale)
  useDensity()
  useMotion()
  const [snap, setSnap] = useState<AppSnapshot | null>(null)
  const [settings, setSettings] = useState<Settings | null>(null)
  const [prop, setProp] = useState<PropagationSnapshot | null>(null)
  const [needAlerts, setNeedAlerts] = useState<NeedAlert[]>([])
  const [bandPlan, setBandPlan] = useState<BandChannel[]>([])
  const [operateLayout, setOperateLayout] = useState<OperateLayout>(loadOperateLayout)
  // Selection mirrors the shared engine (snap.activePeer), so a station picked in the main
  // window — or in this one — highlights consistently across every window.
  const selected = snap?.activePeer ?? null

  // Live snapshot (decodes, stations, radio) — same 300 ms cadence as the main window.
  useEffect(() => subscribeSnapshot(setSnap), [])

  // Refetch the band plan when the tier changes — FT8/FT4 use different dial frequencies
  // (14.074 vs 14.080), so a detached Operate window's QSY targets must follow the mode.
  useEffect(() => {
    let live = true
    getBandPlan().then((b) => live && setBandPlan(b)).catch(() => {})
    return () => {
      live = false
    }
  }, [snap?.link.tier])

  // Propagation + needs + band plan + settings: this window polls the shared engine.
  useEffect(() => {
    let live = true
    const loadProp = () => getPropagation().then((p) => live && setProp(p)).catch(() => {})
    const loadNeeds = () => getNeedAlerts().then((a) => live && setNeedAlerts(a)).catch(() => {})
    // Settings aren't in the snapshot, so poll them too — otherwise a preferRrr / QSO-macro
    // change in the main window never reaches the detached cockpit.
    const loadSettings = () => getSettings().then((s) => live && setSettings(s)).catch(() => {})
    loadProp()
    loadNeeds()
    loadSettings()
    getBandPlan().then((b) => live && setBandPlan(b)).catch(() => {})
    const idP = setInterval(loadProp, 10_000)
    const idN = setInterval(loadNeeds, 15_000)
    const idS = setInterval(loadSettings, 15_000)
    return () => {
      live = false
      clearInterval(idP)
      clearInterval(idN)
      clearInterval(idS)
    }
  }, [])

  // Drive a command then mirror the returned snapshot immediately (the 300 ms poll would
  // catch it anyway, but this keeps the cockpit snappy).
  const apply = (p: Promise<AppSnapshot>) => {
    void p.then((s) => s && setSnap(s)).catch(() => {})
  }

  // `freqMhz` is the spot's exact frequency (source of truth — DXpeditions run off the
  // standard dial); fall back to the band's dial only when the spot has no frequency.
  const qsyBand = (band: string, freqMhz?: number) => {
    const ch = bandPlan.find((c) => c.band === band)
    if (ch) apply(setFrequency(freqMhz ?? ch.dialMhz, ch.band, ch.mode))
  }
  const onSelect = (call: string | null) => {
    // Drives the shared engine; `selected` then reflects it via the snapshot.
    if (call) void selectPeer(call).catch(() => {})
  }
  const onWorkSpot = (t: SpotTarget) => {
    const mode = modeClassOf(t.mode).toLowerCase() as 'cw' | 'phone' | 'digital'
    if (t.freqMhz != null) apply(workSpot(mode, t.freqMhz, t.band, t.call))
    else qsyBand(t.band)
  }
  // Work a decoded/roster station from the cockpit (guards the self-QSO false toast).
  const onCall = (call: string, grid?: string, message?: string, snr?: number, freq?: number) => {
    const me = (snap?.mycall ?? '').trim().toUpperCase().split('/')[0]
    if (me && call.trim().toUpperCase().split('/')[0] === me) return
    apply(callStation(call, grid, message, snr, freq))
  }
  const onTune = (hz: number, target: 'tx' | 'rx' | 'both') => {
    if (target === 'rx') apply(setRxOffset(hz))
    else if (target === 'tx') apply(setTxOffset(hz))
    else apply(setTxOffset(hz).then(() => setRxOffset(hz)))
  }
  const changeLayout = (m: OperateLayout) => {
    setOperateLayout(m)
    try {
      localStorage.setItem('nexus.operate.layout', m)
    } catch {
      /* storage blocked */
    }
  }

  // Connect's map colours stations by need the SAME way the docked map does — gated by the
  // operator's enabled modes (the Needed board has its own per-mode toggles separately).
  const gatedAlerts = useMemo(
    () => visibleNeeds(needAlerts, readEnabledModes()),
    [needAlerts],
  )
  const needByCall = useMemo(() => {
    const m = new Map<string, NeedTag>()
    for (const a of gatedAlerts) if (a.tags.length > 0) m.set(a.call.toUpperCase(), a.tags[0])
    return m
  }, [gatedAlerts])
  const needAlertsByCall = useMemo(() => {
    const m = new Map<string, NeedAlert[]>()
    for (const a of gatedAlerts) {
      const k = a.call.toUpperCase()
      const arr = m.get(k)
      if (arr) arr.push(a)
      else m.set(k, [a])
    }
    return m
  }, [gatedAlerts])

  if (panel === 'needed') {
    return (
      <div className="app detached">
        <NeededPanel
          // Full un-gated list — the board's own mode toggles decide what shows.
          alerts={needAlerts}
          bandPlan={bandPlan}
          selectedCall={selected}
          onQsy={(a) => qsyBand(a.band, a.freqMhz ?? undefined)}
          onSelect={onSelect}
          // Full work path from the pop-out too: the atomic workSpot switches the
          // rig's MODE + exact frequency (a bare QSY left CW clicks in DATA-U),
          // and its snapshot nav-hint (workTick) makes the MAIN window follow to
          // the matching cockpit — this window can't navigate it directly.
          onWork={(a) => {
            const t = workTarget(a, bandPlan)
            if (!t) {
              qsyBand(a.band, a.freqMhz ?? undefined)
              return
            }
            const opMode = t.view === 'operate' ? 'digital' : t.view
            apply(workSpot(opMode, t.freqMhz, t.band, t.call))
          }}
        />
      </div>
    )
  }

  if (panel === 'connect') {
    return (
      <div className="app detached">
        <ConnectView
          myGrid={snap?.mygrid ?? ''}
          theme={theme}
          stations={snap?.stations ?? []}
          prop={prop}
          selectedCall={selected}
          onSelectCall={onSelect}
          needByCall={needByCall}
          onWorkSpot={onWorkSpot}
          needAlerts={gatedAlerts}
          onPoint={
            // Same rotator gate as App; silent fire-and-forget — detached windows
            // have no toast host.
            settings?.rotatorHost?.trim()
              ? (call) => void pointRotatorAtCall(call).catch(() => {})
              : undefined
          }
        />
      </div>
    )
  }

  if (panel === 'dxped') {
    return (
      <div className="app detached">
        <DxpeditionsView snap={prop} onWorkSpot={onWorkSpot} onShowOnMap={onSelect} />
      </div>
    )
  }

  if (panel === 'operate') {
    if (!snap) {
      return (
        <div className="app detached">
          <div className="app loading">
            <span>Connecting to the radio…</span>
          </div>
        </div>
      )
    }
    // The cockpit's Call Roster — a wired StationList. Chat-overlay props (unread, archive)
    // are simplified in the detached Operate window; the roster itself is fully live.
    const roster = (
      <StationList
        stations={snap.stations}
        myGrid={snap.mygrid}
        currentSlot={snap.radio.slot}
        activePeer={selected}
        unreadByPeer={{}}
        needByCall={needByCall}
        onSelect={onSelect}
        onCall={(call) => onCall(call)}
        conversations={snap.conversations as Conv[]}
        onArchive={() => {}}
        bandActive={selected === '*'}
        bandUnread={0}
        onSelectBand={() => onSelect('*')}
      />
    )
    return (
      <div className="app detached operate-detached">
        <OperateCockpit
          snap={snap}
          theme={theme}
          tier={snap.link.tier}
          onTierChange={(t: Tier) => apply(setTier(t))}
          onSourceChange={(k: SourceKind) => apply(setSource(k))}
          onTune={onTune}
          onCall={onCall}
          onSetTxLevel={(lvl: number) => apply(setTxLevel(lvl))}
          onSetMode={(m: ModeRequest) => apply(setMode(m))}
          onSetTxEven={(even: boolean) => apply(setTxEven(even))}
          onSetTxCycleAuto={(auto: boolean) => apply(setTxCycleAuto(auto))}
          onResend={() => apply(qsoResend())}
          onFreetext={(text: string) => apply(qsoFreetext(text))}
          onLog={() => apply(logCurrentQso())}
          onOverrideTx={(call: string, grid: string | null, text: string) =>
            apply(overrideNextTx(call, grid, text))
          }
          onHaltTx={() => apply(haltTx())}
          onSnap={setSnap}
          preferRrr={settings?.preferRrr ?? false}
          qsoMacros={settings?.macros.qso ?? []}
          roster={roster}
          needByCall={needByCall}
          needAlertsByCall={needAlertsByCall}
          selectedCall={selected}
          onSelect={onSelect}
          layoutMode={operateLayout}
          onLayoutMode={changeLayout}
          active
        />
      </div>
    )
  }

  return (
    <div className="app detached">
      <div className="app loading">
        <span>Panel “{panel}” isn’t available as a standalone window yet.</span>
      </div>
    </div>
  )
}
