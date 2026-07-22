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
  SpotRow,
  Tier,
} from './types'
import {
  getBandPlan,
  getNeedAlerts,
  getPropagation,
  getSettings,
  getAllSpots,
  getLog,
  selectPeer,
  archiveConversation,
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
  setTxEnabled,
  setTune,
  setHoldTxFreq,
  dockBandmapWindow,
  setSettings as persistSettings,
} from './api'
import { NeededPanel } from './components/NeededPanel'
import { BandMap } from './components/BandMap'
import { ConnectView } from './components/ConnectView'
import { DxpeditionsView } from './components/DxpeditionsView'
import { SatellitesView } from './components/SatellitesView'
import { Toasts } from './components/Toasts'
import { OperateCockpit } from './components/OperateCockpit'
import { FieldDayScoreboard } from './components/FieldDayView'
import { Waterfall } from './components/Waterfall'
import { StationList } from './components/StationList'
import { visibleNeeds, modeClassOf, workTarget } from './features/needs'
import { OPERATE_PANELS, usePanelLayout } from './features/panelState'
import { surfaceGet, surfaceSet } from './features/windowScope'
import { readEnabledModes } from './useFeatures'
import { useTheme } from './useTheme'
import { useScale } from './useScale'
import { useViewport } from './useViewport'
import { useDensity } from './useDensity'
import { useMotion } from './useMotion'

type SpotTarget = { call: string; band: string; mode: string | null; freqMhz: number | null }
type OperateLayout = 'classic' | 'roster'

// PER-SURFACE, like App's 'nexus.operateLayout'. NB these are two differently-spelled keys
// for one concept and already disagreed before this change — deliberately left as-is here,
// because merging them would alter what the main window reads off disk.
function loadOperateLayout(): OperateLayout {
  // Roster is the default; only an explicit 'classic' choice keeps Classic.
  return surfaceGet('nexus.operate.layout') === 'classic' ? 'classic' : 'roster'
}

export function DetachedPanel({ panel }: { panel: string }) {
  const [theme] = useTheme()
  // A torn-off window is its OWN document — it must publish the same layout/responsive
  // state the main app does, or the CSS falls back to the broken narrow/stacked layout
  // (vertical rails go horizontal, the map collapses to zero height). Mirror App.tsx.
  const { scale } = useScale()
  useViewport(scale)
  useDensity()
  useMotion()
  const [snap, setSnap] = useState<AppSnapshot | null>(null)
  const [settings, setSettings] = useState<Settings | null>(null)
  // Waterfall pop-out ⇄ dock: while this torn-off waterfall window lives, the main cockpit hides
  // its docked copy so the decode lists + roster get the room. On close (or unmount) we clear the
  // flag; the main window's `storage` listener then re-docks automatically. The main cockpit also
  // has an always-visible manual "re-dock" as the fallback if this never fires.
  useEffect(() => {
    if (panel !== 'waterfall') return
    const KEY = 'nexus.waterfall.detached'
    localStorage.setItem(KEY, '1')
    const clear = () => localStorage.setItem(KEY, '0')
    window.addEventListener('beforeunload', clear)
    return () => {
      clear()
      window.removeEventListener('beforeunload', clear)
    }
  }, [panel])
  const [prop, setProp] = useState<PropagationSnapshot | null>(null)
  const [needAlerts, setNeedAlerts] = useState<NeedAlert[]>([])
  const [bandPlan, setBandPlan] = useState<BandChannel[]>([])
  const [operateLayout, setOperateLayout] = useState<OperateLayout>(loadOperateLayout)
  // This window is its OWN surface (instance `w1` by default), so its ⊞ Panels choices
  // are independent of the docked cockpit's — that is the whole point of keying the
  // record per surface instead of one app-global flag.
  const operatePanels = usePanelLayout(OPERATE_PANELS)
  // Band-map pop-out only: the live spot feed + which calls are in the log (worked).
  const isBandMap = panel === 'bandmapPhone' || panel === 'bandmapCw'
  const [allSpots, setAllSpots] = useState<SpotRow[]>([])
  const [workedCalls, setWorkedCalls] = useState<Set<string>>(() => new Set())
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

  // Band-map pop-out: poll the live spot feed + refresh the worked-set (log calls) alongside it.
  useEffect(() => {
    if (!isBandMap) return
    let live = true
    const load = () => {
      getAllSpots().then((s) => live && setAllSpots(s)).catch(() => {})
      getLog()
        .then((log) => live && setWorkedCalls(new Set(log.map((q) => q.call.toUpperCase()))))
        .catch(() => {})
    }
    load()
    const id = setInterval(load, 15_000)
    return () => {
      live = false
      clearInterval(id)
    }
  }, [isBandMap])

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
    // `null` is a real command — it clears the engine's active peer (deselect on
    // empty-map click / ✕ / re-click a dot); swallowing it left selection stuck.
    void selectPeer(call).catch(() => {})
  }
  // Mirrors App.tsx's handleArchive — the detached window had a silent no-op here, so the
  // ✕ did nothing at all in this panel.
  const onArchive = (peer: string) => {
    if (
      !window.confirm(
        `Delete the conversation with ${peer}? Any messages still waiting to send will be cancelled. This can't be undone.`,
      )
    )
      return
    apply(archiveConversation(peer))
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
    surfaceSet('nexus.operate.layout', m)
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
  const typeByCall = useMemo(() => {
    const m = new Map<string, 'Pota' | 'Sota' | 'Dxped'>()
    for (const a of gatedAlerts) {
      const k = a.call.toUpperCase()
      if (m.has(k)) continue
      const t = a.tags.find((x) => x === 'Pota' || x === 'Sota' || x === 'Dxped')
      if (t) m.set(k, t as 'Pota' | 'Sota' | 'Dxped')
    }
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

  if (isBandMap) {
    if (!snap) return <div className="app detached" />
    const spotMode: 'CW' | 'Phone' = panel === 'bandmapCw' ? 'CW' : 'Phone'
    return (
      <div className="app detached">
        <BandMap
          band={snap.radio.band}
          dialMhz={snap.radio.dialMhz}
          txAllowed={snap.radio.txAllowed}
          // Phone-segment shade is meaningless on the CW map (matches the inline CW strip).
          phoneSegLo={spotMode === 'Phone' ? snap.radio.phoneSegLo : null}
          phoneSegHi={spotMode === 'Phone' ? snap.radio.phoneSegHi : null}
          spots={allSpots}
          spotMode={spotMode}
          needByCall={needByCall}
          typeByCall={typeByCall}
          workedCalls={workedCalls}
          onDock={(side) => void dockBandmapWindow(side)}
          onWorkSpot={(s) =>
            onWorkSpot({ call: s.call, band: s.band, mode: s.mode, freqMhz: s.freqMhz })
          }
        />
      </div>
    )
  }

  if (panel === 'waterfall') {
    // The FT8/digital waterfall, torn off — it self-fetches its spectrum; clicks tune
    // the shared engine's RX/TX offsets exactly like the in-cockpit strip.
    return (
      <div className="detached detached-waterfall">
        <Waterfall
          transmitting={snap?.radio.transmitting ?? false}
          rxOffsetHz={snap?.radio.rxOffsetHz ?? 1500}
          txOffsetHz={snap?.radio.txOffsetHz ?? 1500}
          theme={theme}
          onTune={(hz, target) => {
            if (target === 'rx' || target === 'both') void setRxOffset(hz)
            if (target === 'tx' || target === 'both') void setTxOffset(hz)
          }}
          active
        />
        <Toasts />
      </div>
    )
  }

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
            // The board lists ALL modes, but the CW/Phone cockpits are opt-in features.
            // If the target cockpit is disabled, the MAIN window's nav-hint effect refuses
            // to follow (same gate as handleWorkNeeded) — so a workSpot would silently
            // switch the rig into a hidden mode with no UI. Just QSY to the spot instead.
            const modes = readEnabledModes()
            if ((t.view === 'cw' && !modes.cw) || (t.view === 'phone' && !modes.phone)) {
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
            // Same rotator gate as App (model-launched rotctld OR external host);
            // silent fire-and-forget — detached windows have no toast host.
            (settings?.rotatorModel ?? 0) > 0 || settings?.rotatorHost?.trim()
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

  if (panel === 'sats') {
    return (
      <div className="app detached">
        <SatellitesView />
        {/* This panel's Track/alarm actions report via toasts — unlike the older
            detached panels (silent by design), it needs a host in this window. */}
        <Toasts />
      </div>
    )
  }

  if (panel === 'fieldday') {
    const fd = snap?.fieldDay ?? null
    return (
      <div className="app detached">
        {fd ? (
          <FieldDayScoreboard
            fieldDay={fd}
            settings={settings}
            detached
            onSaveOperator={(call) => {
              if (!settings) return
              const updated: Settings = { ...settings, fdOperator: call }
              setSettings(updated) // optimistic local mirror (useState)
              apply(persistSettings(updated)) // persist + mirror the returned snapshot
            }}
          />
        ) : (
          <div className="app loading">
            <span>Field Day isn’t active.</span>
          </div>
        )}
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
        needAlertsByCall={needAlertsByCall}
        onSelect={onSelect}
        onCall={(call) => onCall(call)}
        conversations={snap.conversations as Conv[]}
        onArchive={onArchive}
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
          bandPlan={bandPlan}
          onSetFrequency={(dialMhz: number, band: string, mode: string) =>
            apply(setFrequency(dialMhz, band, mode))
          }
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
          onSetTxEnabled={(on: boolean) => apply(setTxEnabled(on))}
          onSetTune={(on: boolean) => apply(setTune(on))}
          onSetHoldTxFreq={(on: boolean) => apply(setHoldTxFreq(on))}
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
          panels={operatePanels}
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
