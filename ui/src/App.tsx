import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { AppSnapshot, BandChannel, LoggedQso, ModeRequest, Settings, SourceKind, Tier } from './types'
import {
  callStation as apiCallStation,
  overrideNextTx as apiOverrideNextTx,
  setArea as apiSetArea,
  openPanelWindow,
  qsoResend as apiQsoResend,
  qsoFreetext as apiQsoFreetext,
  logCurrentQso as apiLogCurrentQso,
  confirmPendingLog as apiConfirmPendingLog,
  discardPendingLog as apiDiscardPendingLog,
  getBandPlan,
  getSettings,
  getSnapshot,
  selectPeer as apiSelectPeer,
  archiveConversation as apiArchiveConversation,
  sendMessage as apiSendMessage,
  broadcast as apiBroadcast,
  callCq as apiCallCq,
  setFrequency as apiSetFrequency,
  setMode as apiSetMode,
  setTier as apiSetTier,
  setSource as apiSetSource,
  setTxEnabled as apiSetTxEnabled,
  setTxLevel as apiSetTxLevel,
  setTune as apiSetTune,
  haltTx as apiHaltTx,
  setTxEven as apiSetTxEven,
  setTxCycleAuto as apiSetTxCycleAuto,
  setBeacon as apiSetBeacon,
  setRxOffset as apiSetRxOffset,
  setTxOffset as apiSetTxOffset,
  setHoldTxFreq as apiSetHoldTxFreq,
  subscribeSnapshot,
} from './api'
import { withErrorToast, pushToast } from './toast'
import { processDecodes } from './alerts'
import { useTheme } from './useTheme'
import { useLayout } from './useLayout'
import { useScale } from './useScale'
import { useViewport } from './useViewport'
import { useDensity } from './useDensity'
import { useMotion } from './useMotion'
import { useAchievements } from './useAchievements'
import { useJourneyUnlocks } from './useJourneyUnlocks'
import { useFeatures } from './useFeatures'
import { useReveals } from './useReveals'
import { sectionFeatures, featureById, type FeatureId } from './features/registry'
import { visibleNeeds, workTarget, modeClassOf } from './features/needs'
import { usePaneWidths, clampLeft, clampRight } from './usePaneWidths'
import { TopBar } from './components/TopBar'
import { StationList } from './components/StationList'
import { Conversation } from './components/Conversation'
import { Waterfall } from './components/Waterfall'
import { LinkPill } from './components/LinkPill'
import { ModeNav, type View, type DigitalMode } from './components/ModeNav'
import { OperateCockpit } from './components/OperateCockpit'
import { NowBar } from './components/NowBar'
import { AwardsJourney } from './components/AwardsJourney'
import { CwCockpit } from './components/CwCockpit'
import { PhoneCockpit } from './components/PhoneCockpit'
import { PotaSotaView, type OtaSpotClickArg } from './components/PotaSotaView'
import { DxpeditionsView } from './components/DxpeditionsView'
import { ConnectView } from './components/ConnectView'
import {
  getPropagation,
  getFeedHealth,
  getNeedAlerts,
  getAllSpots,
  getXrayNow,
  getDxpedWindows,
  setOperatingMode,
  workSpot,
  setLicenseClass,
  stopQsoRecording,
  pointRotatorAtCall,
  qsySetEnabled as apiQsySetEnabled,
} from './api'
import { processFlare, effectiveXray } from './flareAlert'
import { processDxpedAlerts } from './features/dxpedChase'
import { dxpedWorkMode } from './components/connect/paneFormat'
import { setStatus } from './status'
import type { PropagationSnapshot, FeedHealth, NeedTag, NeedAlert, SpotRow, DxpedWindow, WorkableCard } from './types'
import { NeededPanel } from './components/NeededPanel'
import { SpotsPanel } from './components/SpotsPanel'
import { LogConfirm } from './components/LogConfirm'
import { FieldDayView } from './components/FieldDayView'
import { OperateDecodes } from './components/OperateDecodes'
import { Logbook } from './components/Logbook'
import { RoamPanel } from './components/RoamPanel'
import { SettingsPanel } from './components/SettingsPanel'
import { Toasts } from './components/Toasts'
import { OnboardingBanner } from './components/OnboardingBanner'
import { RevealNudge } from './components/RevealNudge'
import { SetupWizard } from './components/SetupWizard'
import type { ProfileId } from './features/profiles'

const ONBOARD_KEY = 'tempo-onboarded'
// First-run setup wizard: shown once on a fresh install, re-openable from Settings.
const WIZARD_KEY = 'nexus.features.wizardSeen'
function wizardSeen(): boolean {
  try {
    return localStorage.getItem(WIZARD_KEY) === '1'
  } catch {
    return true // storage blocked → don't nag
  }
}
function markWizardSeen(): void {
  try {
    localStorage.setItem(WIZARD_KEY, '1')
  } catch {
    /* storage blocked — wizard simply won't persist as seen */
  }
}
// Read-only storage (e.g. Safari private mode): writes throw while reads succeed,
// so "seen" can't persist. Suppress the wizard there so it can't re-nag on every
// reload — it'd be shown forever otherwise.
function storageWritable(): boolean {
  try {
    const k = 'nexus.__probe'
    localStorage.setItem(k, '1')
    localStorage.removeItem(k)
    return true
  } catch {
    return false
  }
}

// Macros fall back to these until settings load (keeps chips populated).
const DEFAULT_MACROS: Settings['macros'] = {
  // 'chat' chips are DIRECTED replies to the selected peer. (CQ moved to 'band':
  // calling CQ is a broadcast, not a message to one station.)
  chat: ['73', 'QSL', 'Name?', 'QTH?', 'GE'],
  qso: ['R-09', 'RRR', 'RR73', '73'],
  // 'band' chips are open free-text BROADCASTS. A CQ goes through the structured
  // Call-CQ button (not a free-text chip, which chunked into a gridless "A12CQ").
  band: ['QRZ?', 'PSE K', '73', 'GL'],
}

export default function App() {
  const [theme, setTheme] = useTheme()
  const [wfLayout, setWfLayout] = useLayout()
  const [scale, setScale] = useScale()
  // Publishes the zoom-aware `data-viewport` size class on <html> (live on resize
  // AND on scale change) so the layout adapts to the EFFECTIVE width.
  useViewport(scale)
  // Activates + persists the density + motion attributes (control UI lands later).
  useDensity()
  useMotion()
  // Modular features (toggles + profiles). Drives nav, view-gating, and the
  // gamification/achievements layer.
  const features = useFeatures()
  useAchievements(features.isOn('gamification'))
  useJourneyUnlocks(features.isOn('gamification'))
  const reveal = useReveals(features)
  // First-run setup wizard (goal-driven). Only on a genuinely fresh install.
  const [showWizard, setShowWizard] = useState<boolean>(
    () => features.firstRun && storageWritable() && !wizardSeen(),
  )
  const { commitLeft, commitRight, resetWidths } = usePaneWidths()
  const layoutRef = useRef<HTMLElement>(null)
  const [snap, setSnap] = useState<AppSnapshot | null>(null)
  // Live mirror of our callsign for callbacks with empty dep lists (handleCall
  // guards against working yourself without re-creating the callback per snap).
  const mycallRef = useRef('')
  useEffect(() => {
    mycallRef.current = snap?.mycall ?? ''
  }, [snap?.mycall])
  // Click-to-work handoff: a Needed-board click on a voice/CW spot seeds this, the
  // matching cockpit consumes it to prefill the log. `ts` makes a re-click of the same
  // call refire the cockpit's prefill effect. Cleared once consumed.
  const [pendingWork, setPendingWork] = useState<{
    call: string
    view: 'cw' | 'phone'
    ts: number
  } | null>(null)
  // Roam settings panel (inside the Tempo cockpit) open/closed.
  const [roamOpen, setRoamOpen] = useState(false)
  const [view, setView] = useState<View>(() => {
    const h = typeof window !== 'undefined' ? window.location.hash.slice(1) : ''
    const sectionIds = sectionFeatures().map((f) => f.id) as string[]
    // Honor a deeplink only if it's an enabled section; otherwise open at the
    // active profile's landing view.
    if (sectionIds.includes(h) && features.enabled[h as FeatureId] !== false) {
      return h as View
    }
    // Merged sections — honor old deeplinks.
    if (h === 'propagation' || h === 'map') return 'connect'
    return features.landing
  })
  // Per-section rig-mode policy. Only ENTERING an actual operating cockpit changes the rig:
  // the workspace sections (FT8/FT4, Tempo, contest…) + the global CW/Phone cockpits. A
  // global, non-operating view (Map, Logbook, Settings, Propagation, Awards…) leaves the rig
  // exactly as the last operating section set it — glancing at the map mid-CW-QSO must never
  // touch the VFO or mode, and (crucially) must not advance the guard, so a later Operate
  // click still QSYs. `followFreq` is true only for the three explicit mode tabs (Phone / CW /
  // Digital-Operate) — entering one drops the rig to that mode's home freq; the other digital
  // cockpits (chat/qso/…) set the mode only and keep their own band picker's frequency.
  const lastOpModeRef = useRef<'digital' | 'phone' | 'cw'>('digital')
  useEffect(() => {
    const operating = !!featureById(view as FeatureId)?.workspace || view === 'cw' || view === 'phone'
    if (!operating) return
    const mode = view === 'cw' ? 'cw' : view === 'phone' ? 'phone' : 'digital'
    // ALWAYS (re)assert the rig mode on entering an operating view. We must NOT skip it with a
    // same-value guard: the guard ref drifts out of sync with the real rig (handleDigitalMode
    // and the Needed click set the mode without going through here), which left the rig stuck
    // in the wrong mode while the VFO read-back kept working. The backend is idempotent and
    // re-arms an immediate retune, so re-asserting is cheap. Only RE-HOME the frequency on a
    // genuine mode change, so returning to a mode you were already in never yanks the VFO.
    const changed = mode !== lastOpModeRef.current
    lastOpModeRef.current = mode
    const followFreq = changed && (view === 'operate' || view === 'cw' || view === 'phone')
    void setOperatingMode(mode, followFreq)
      .then((s) => s && setSnap(s))
      .catch(() => {})
  }, [view])
  const [prop, setProp] = useState<PropagationSnapshot | null>(null)
  // Operate layout mode: Classic (WSJT-X — Band Activity dominant) vs Roster
  // (GridTracker — the Call Roster dominant). Persisted UI pref; new hams who
  // love GridTracker pick Roster, die-hards keep Classic. Default Classic.
  const [operateLayout, setOperateLayout] = useState<'classic' | 'roster'>(() => {
    try {
      const v = localStorage.getItem('nexus.operateLayout')
      if (v === 'classic' || v === 'roster') return v
    } catch {
      /* unreadable storage — fall through to default */
    }
    return 'classic'
  })
  const handleOperateLayout = useCallback((m: 'classic' | 'roster') => {
    setOperateLayout(m)
    try {
      localStorage.setItem('nexus.operateLayout', m)
    } catch {
      /* ignore persist failure */
    }
  }, [])

  // Operate MODE: 'dx' (FT8/FT4 structured cockpit) or 'msg' (Tempo two-way
  // calling). The FT8/FT4 ⇄ Tempo switch binds the radio tier+mode and swaps only
  // the cockpit; Connect/Map/Prop/Logbook/Awards are GLOBAL views selected from the
  // sidebar (they never retune the radio). Default FT8/FT4 (the 80% case).
  const [area, setArea] = useState<'dx' | 'msg'>(() => {
    try {
      const v = localStorage.getItem('nexus.workspace')
      if (v === 'dx' || v === 'msg') return v
      // Migrate the retired 'connect' area to FT8/FT4 (Connect is now a global view).
    } catch {
      /* unreadable — fall through */
    }
    return 'dx'
  })
  // Sync the engine to the persisted mode once on load (atomic tier+mode).
  const areaSyncedRef = useRef(false)
  useEffect(() => {
    if (areaSyncedRef.current || !snap) return
    areaSyncedRef.current = true
    void apiSetArea(area).then((s) => s && setSnap(s))
    // Reconcile the cockpit view with the mode (a persisted Tempo mode must not
    // open on the FT8/FT4 cockpit, and vice-versa). Global views are left alone.
    setView((v) =>
      area === 'msg' && v === 'operate' ? 'chat' : area === 'dx' && v === 'chat' ? 'operate' : v,
    )
  }, [snap, area])

  // Pop the Needed board out into its own window on every app load (the operator can
  // close it). Once per launch, after the app is up + only if the feature is enabled;
  // a no-op in the browser/mock (the Tauri command isn't there) and in a detached
  // panel window (that renders DetachedPanel, not App).
  const neededPoppedRef = useRef(false)
  useEffect(() => {
    if (neededPoppedRef.current || !snap) return
    if (features.enabled.needed === false) return
    neededPoppedRef.current = true
    void openPanelWindow('needed').catch(() => {})
  }, [snap, features.enabled])

  const handleWorkspace = useCallback((w: 'dx' | 'msg') => {
    setArea(w)
    try {
      localStorage.setItem('nexus.workspace', w)
    } catch {
      /* ignore */
    }
    // Switching mode lands on that mode's cockpit (FT8/FT4 → Operate, Tempo → Chat).
    setView(w === 'dx' ? 'operate' : 'chat')
    void withErrorToast(() => apiSetArea(w), 'Could not switch mode').then((s) => {
      if (s) setSnap(s)
    })
  }, [])
  // Surface a dead radio engine (audio_error) in the persistent status lane —
  // it was only visible deep in Settings ▸ CAT, i.e. effectively invisible.
  useEffect(() => {
    const err = snap?.radio.audioError
    if (err) {
      setStatus('audio', { tier: 'critical', message: 'RADIO STOPPED', detail: err })
    } else {
      setStatus('audio', null)
    }
  }, [snap?.radio.audioError])

  // Connector auto-upload outcomes (QRZ/ClubLog/eQSL) now happen in the backend
  // log funnel; the engine bumps uploadTick per outcome and we toast it here —
  // the operator SEES every upload land (or fail) regardless of which path
  // logged the QSO (the auto-logged FT8 path included).
  const uploadTickRef = useRef(0)
  useEffect(() => {
    const tick = snap?.uploadTick ?? 0
    if (tick !== uploadTickRef.current) {
      uploadTickRef.current = tick
      if (snap?.uploadNote) {
        pushToast(snap.uploadNote, snap.uploadOk ? 'success' : 'error')
      }
    }
  }, [snap?.uploadTick, snap?.uploadNote, snap?.uploadOk])


  // Per-(band,mode) last-alert time so a band coming alive toasts once, not every
  // poll (defence in depth — the backend tracker already flags `isNew` once).
  const openingAlertRef = useRef<Map<string, number>>(new Map())
  // Freshest fast-lane X-ray reading (60 s poller below) — merged with each prop
  // snapshot so the flare heads-up fires app-wide, whatever view is open.
  const xrayFastRef = useRef<number | null>(null)
  // Chased-DXpedition alert inputs: the latest windows sweep (10-min poller
  // below), the current QSO partner (kept fresh by the decode-alert effect),
  // and the work action (assigned once handleWorkMapSpot exists).
  const dxpedWindowsRef = useRef<Map<string, DxpedWindow> | null>(null)
  const qsoPartnerRef = useRef<string | null>(null)
  const workDxpedRef = useRef<((c: WorkableCard) => void) | null>(null)
  // Latest link tier (FT1/DX1/FT8/FT4) — the rail's Digital button preserves it.
  const tierRef = useRef<Tier>('FT8')
  useEffect(() => {
    let live = true
    const OPENING_ALERT_COOLDOWN_MS = 10 * 60_000
    const load = () =>
      getPropagation()
        .then((p) => {
          if (!live) return
          setProp(p)
          // Solar-flare heads-up (edge-triggered; flareAlert.ts owns the dedup).
          processFlare(effectiveXray(xrayFastRef.current, p.spaceWx.xrayLong))
          // Chased-expedition window alerts (dxpedChase.ts owns the dedup).
          processDxpedAlerts(
            p.dxpeditions.workableNow,
            dxpedWindowsRef.current,
            qsoPartnerRef.current,
            (c) => workDxpedRef.current?.(c),
          )
          // Loud one-shot alert when a band comes alive (the flagship moment).
          const tnow = Date.now()
          for (const o of p.openings) {
            if (!o.isNew) continue
            const key = `${o.band}|${o.mode}`
            const last = openingAlertRef.current.get(key) ?? 0
            if (tnow - last < OPENING_ALERT_COOLDOWN_MS) continue
            openingAlertRef.current.set(key, tnow)
            pushToast(
              `⚡ ${o.band} open — ${o.mode} · point ${o.octant} · ${o.stations} stns`,
              'success',
              8000,
            )
          }
          // Honest-state: surface non-live propagation in the Now-Bar lane.
          if (p.source === 'offline') {
            setStatus('prop', {
              tier: 'warning',
              message: 'Prop: no live data',
              detail:
                'No live propagation data yet — set your callsign in Settings and check your internet connection.',
            })
          } else if (p.source === 'cached') {
            const ageMin = Math.max(0, Math.round((Date.now() / 1000 - p.asOf) / 60))
            setStatus('prop', {
              tier: 'warning',
              message: `Prop: cached ${ageMin}m`,
              detail: 'Live propagation refetch failed — showing the last-good snapshot.',
            })
          } else {
            setStatus('prop', null)
          }
        })
        .catch(() => {})
    load()
    const id = setInterval(load, 30_000)
    return () => {
      live = false
      clearInterval(id)
    }
  }, [])
  // X-ray fast lane (60 s): flare ONSET reaches the operator in ~1 min instead of
  // the 5-min prop-snapshot cadence. Best-effort — a failed fetch just leaves the
  // snapshot's slower value driving the watcher.
  useEffect(() => {
    let live = true
    const load = () =>
      getXrayNow()
        .then((x) => {
          if (!live) return
          xrayFastRef.current = x.flux
          processFlare(effectiveXray(x.flux, null))
        })
        .catch(() => {})
    load()
    const id = setInterval(load, 60_000)
    return () => {
      live = false
      clearInterval(id)
    }
  }, [])
  // DXpedition best-shot windows for the chase alerts (server-cached climatology;
  // 10 min is generous). Best-effort — without it the loud spotted-alert still
  // works from the snapshot's cards; only the quiet modelled-only toast needs it.
  useEffect(() => {
    let live = true
    const load = () =>
      getDxpedWindows()
        .then((list) => {
          if (live) dxpedWindowsRef.current = new Map(list.map((w) => [w.call.toUpperCase(), w]))
        })
        .catch(() => {})
    load()
    const id = setInterval(load, 600_000)
    return () => {
      live = false
      clearInterval(id)
    }
  }, [])
  // Live-feed liveness for the Now-Bar connector pills (same cadence as prop).
  const [feedHealth, setFeedHealth] = useState<FeedHealth | null>(null)
  useEffect(() => {
    let live = true
    const load = () =>
      getFeedHealth()
        .then((h) => live && setFeedHealth(h))
        .catch(() => {})
    load()
    const id = setInterval(load, 30_000)
    return () => {
      live = false
      clearInterval(id)
    }
  }, [])
  const [needAlerts, setNeedAlerts] = useState<NeedAlert[]>([])
  useEffect(() => {
    let live = true
    const load = () =>
      getNeedAlerts()
        .then((alerts) => {
          if (live) setNeedAlerts(alerts)
        })
        .catch(() => {})
    load()
    const id = setInterval(load, 30_000)
    return () => {
      live = false
      clearInterval(id)
    }
  }, [])
  // Raw spot firehose for the Spots panel (ungated, all modes). Polled faster than needs
  // since it's a live "what's on the air" view; the backend command just reads the buffer.
  const [allSpots, setAllSpots] = useState<SpotRow[]>([])
  useEffect(() => {
    let live = true
    const load = () =>
      getAllSpots()
        .then((s) => {
          if (live) setAllSpots(s)
        })
        .catch(() => {})
    load()
    const id = setInterval(load, 15_000)
    return () => {
      live = false
      clearInterval(id)
    }
  }, [])
  // Gate CW/Phone needs by the operator's enabled modes — the backend emits voice/CW
  // needs unconditionally, visibility is the frontend's call. A pure-digital op's board,
  // roster colouring, and map highlight all derive from THIS gated set, so they stay
  // exactly as before the feature.
  const cwEnabled = features.isOn('cw')
  const phoneEnabled = features.isOn('phone')
  // Work-a-spot navigation: whenever a spot is worked — from THIS window's
  // boards or a pop-out (which can't navigate the main window itself) — follow
  // to the matching cockpit (the operator's report: "if I click a contact, it
  // should bring me into the right section"). Baselined on the first snapshot
  // so a webview reload never replays the engine's last work action.
  const workNavRef = useRef<number | null>(null)
  useEffect(() => {
    const tick = snap?.workTick ?? 0
    if (workNavRef.current === null) {
      workNavRef.current = tick
      return
    }
    if (tick === workNavRef.current) return
    workNavRef.current = tick
    const v = snap?.workView
    const target: View = v === 'cw' ? 'cw' : v === 'phone' ? 'phone' : 'operate'
    // Never navigate into a feature-disabled (hidden) cockpit — same gate as
    // handleWorkNeeded; the rig already switched, the view just stays put.
    if ((target === 'cw' && !cwEnabled) || (target === 'phone' && !phoneEnabled)) return
    // Sync the rig-mode guard BEFORE navigating (same as handleWorkNeeded) —
    // otherwise the [view] effect sees a mode change and re-homes the dial to
    // the segment start, yanking the rig OFF the exact spot frequency the
    // workSpot click just tuned (review-caught on the pop-out path).
    lastOpModeRef.current = target === 'operate' ? 'digital' : target
    setView(target)
  }, [snap?.workTick, snap?.workView, cwEnabled, phoneEnabled])
  const visibleAlerts = useMemo(
    () => visibleNeeds(needAlerts, { cw: cwEnabled, phone: phoneEnabled }),
    [needAlerts, cwEnabled, phoneEnabled],
  )
  // Need-tier per heard call (top tag) for roster/map colouring — from the GATED set so
  // a disabled mode never colours a station the board hides.
  const needByCall = useMemo(() => {
    const m = new Map<string, NeedTag>()
    for (const a of visibleAlerts) {
      if (a.tags.length > 0) m.set(a.call.toUpperCase(), a.tags[0])
    }
    return m
  }, [visibleAlerts])
  // Full alert set per heard call (all bands/modes) for the band-activity decode feed's
  // need-icons + row colouring — richer than needByCall's top-tag-only map. Keyed
  // UPPERCASE; from the same GATED visibleAlerts so a disabled mode never tags a row.
  const needAlertsByCall = useMemo(() => {
    const m = new Map<string, NeedAlert[]>()
    for (const a of visibleAlerts) {
      const k = a.call.toUpperCase()
      const arr = m.get(k)
      if (arr) arr.push(a)
      else m.set(k, [a])
    }
    return m
  }, [visibleAlerts])
  const [typingTick, setTypingTick] = useState(0)
  const [bandPlan, setBandPlan] = useState<BandChannel[]>([])
  const [settings, setSettings] = useState<Settings | null>(null)
  const [onboardDismissed, setOnboardDismissed] = useState<boolean>(
    () => localStorage.getItem(ONBOARD_KEY) === '1',
  )

  // Track how many messages we'd "read" per peer to compute unread counts.
  const readCounts = useRef<Record<string, number>>({})

  const reloadSettings = useCallback(() => {
    getSettings().then(setSettings).catch(() => {})
  }, [])

  // initial load + live subscription
  useEffect(() => {
    let mounted = true
    getSnapshot().then((s) => mounted && setSnap(s))
    getBandPlan()
      .then((b) => mounted && setBandPlan(b))
      .catch(() => {})
    getSettings()
      .then((s) => mounted && setSettings(s))
      .catch(() => {})
    const unsub = subscribeSnapshot((s) => {
      if (mounted) setSnap(s)
    })
    return () => {
      mounted = false
      unsub()
    }
  }, [])

  // Refetch the band plan when the tier changes: FT8/FT4 use the standard WSJT-X
  // watering holes (14.074 …), FT1/DX1 the native off-cluster plan. So picking a
  // band always lands you where that mode actually calls.
  const activeTier = snap?.link.tier
  useEffect(() => {
    let live = true
    getBandPlan()
      .then((b) => live && setBandPlan(b))
      .catch(() => {})
    return () => {
      live = false
    }
  }, [activeTier])

  // Periodic re-eval ticker so the unread badges refresh smoothly between snapshot
  // polls (the unread memos read a ref cursor that a dep change alone won't catch).
  useEffect(() => {
    const id = window.setInterval(() => setTypingTick((t) => t + 1), 400)
    return () => window.clearInterval(id)
  }, [])


  const activePeer = snap?.activePeer ?? null

  // mark the active conversation as read whenever it updates
  useEffect(() => {
    if (!snap) return
    // Prune read-cursors for threads that no longer exist (archived, or rebuilt) so
    // a re-created thread starts unread from 0 instead of inheriting a stale cursor
    // that would collapse its unread count to zero.
    const live = new Set(snap.conversations.map((c) => c.peer))
    for (const k of Object.keys(readCounts.current)) {
      if (!live.has(k)) delete readCounts.current[k]
    }
    if (!activePeer) return
    const conv = snap.conversations.find((c) => c.peer === activePeer)
    if (conv) readCounts.current[activePeer] = conv.messages.length
  }, [snap, activePeer])

  const unreadByPeer = useMemo(() => {
    const out: Record<string, number> = {}
    if (!snap) return out
    for (const c of snap.conversations) {
      // the "*" broadcast peer is an engine-internal bus; skip unread badges
      if (c.peer === '*') continue
      const read = readCounts.current[c.peer] ?? 0
      const inbound = c.messages.filter((m) => !m.outbound).length
      const readInbound = Math.min(read, c.messages.length)
      // unread = inbound messages beyond what we've seen
      const unread = c.messages.slice(readInbound).filter((m) => !m.outbound).length
      if (c.peer !== activePeer && unread > 0) out[c.peer] = unread
      void inbound
    }
    return out
  }, [snap, activePeer, typingTick])

  // Unread on the "*" band feed (CQs/broadcasts from others). Tracked separately
  // from unreadByPeer (which is per-station) and shown on the pinned Band row; the
  // read cursor is bumped by the same effect above when "*" is the active peer.
  const bandUnread = useMemo(() => {
    if (!snap || activePeer === '*') return 0
    const band = snap.conversations.find((c) => c.peer === '*')
    if (!band) return 0
    const read = readCounts.current['*'] ?? 0
    const readInbound = Math.min(read, band.messages.length)
    return band.messages.slice(readInbound).filter((m) => !m.outbound).length
  }, [snap, activePeer, typingTick])

  const handleSelect = useCallback((call: string) => {
    void withErrorToast(() => apiSelectPeer(call), 'Could not select station').then(
      (s) => s && setSnap(s),
    )
  }, [])

  const handleArchive = useCallback((peer: string) => {
    void withErrorToast(
      () => apiArchiveConversation(peer),
      'Could not archive conversation',
    ).then((s) => s && setSnap(s))
  }, [])

  // The Map and the roster share ONE selection: the active peer. Clicking a map
  // dot selects (or, if already selected, clears) that station — and the roster
  // highlights it too, since StationList already keys its highlight off activePeer.
  const handleMapSelect = useCallback((call: string | null) => {
    void withErrorToast(() => apiSelectPeer(call), 'Could not select station').then(
      (s) => s && setSnap(s),
    )
  }, [])

  const handleCall = useCallback(
    (call: string, grid?: string, message?: string, snr?: number, freq?: number) => {
      // Clicking your OWN line (your CQ / TX echo): the engine guards against
      // the self-QSO, but without this the command still returns a snapshot and
      // we'd flash a FALSE "Working KD9TAW" success toast.
      const me = mycallRef.current.trim().toUpperCase()
      if (me && call.trim().toUpperCase().split('/')[0] === me.split('/')[0]) {
        pushToast(`${call} is your own call`, 'info', 2500)
        return
      }
      void withErrorToast(
        () => apiCallStation(call, grid, message, snr, freq),
        `Could not work ${call}`,
      ).then((s) => {
        if (s) {
          setSnap(s)
          // Work the station on the single-screen Operate cockpit — the QSO
          // sequences inline there while the waterfall + decodes stay visible.
          // (Never bounce to the chat-style 'qso' view and lose the band.)
          setView('operate')
          // Immediate confirmation the action took (and TX is now armed for it).
          pushToast(`▶ Working ${call} — transmitting your call`, 'success', 4000)
        }
      })
    },
    [],
  )

  // Fire decode alerts (beep + toast) whenever the decode feed changes, gated by the
  // user's alert settings. processDecodes dedups internally. The third arg makes each
  // alert toast click-to-work — working the station is what you almost always want next
  // (someone calling you, a new DXCC/grid, a CQ). Placed AFTER handleCall so it's in scope.
  // The QSO context keeps popups quiet while actively working someone / running CQ.
  useEffect(() => {
    if (!snap || !settings) return
    const dxcall = snap.fieldDay?.dxcall ?? snap.qso?.dxcall ?? null
    // Keep the chase-alert suppression in sync (the prop poller reads the ref).
    qsoPartnerRef.current = dxcall
    // Latest tier for the rail's Digital button (preserve FT4 across nav).
    tierRef.current = snap.link.tier
    processDecodes(
      snap.recentDecodes,
      settings,
      (d) => {
        if (d.from) handleCall(d.from, undefined, d.message, d.snr, d.freqHz)
      },
      // Field Day runs its own sequencer (snap.qso is null there) — its state
      // strings (CallingCq/AwaitExchange/AwaitConfirm/Done) gate identically.
      {
        state: snap.fieldDay?.state ?? snap.qso?.state ?? null,
        dxcall,
      },
    )
  }, [snap, settings, handleCall])

  // Bumps when a QSO is logged AND "Clear DX call after logging" is on — the
  // cockpit watches it and wipes its DX Call/Grid fields (stock WSJT-X option).
  const [dxClearTick, setDxClearTick] = useState(0)
  const noteLoggedForDxClear = useCallback(() => {
    if (settings?.clearDxAfterLog) setDxClearTick((t) => t + 1)
  }, [settings?.clearDxAfterLog])

  const handleConfirmLog = useCallback(
    (record: LoggedQso) => {
      void withErrorToast(() => apiConfirmPendingLog(record), 'Could not log QSO').then((s) => {
        if (s) {
          setSnap(s)
          noteLoggedForDxClear()
          // QRZ/ClubLog/eQSL auto-upload happens in the BACKEND log funnel now
          // (every log path, auto-log included); outcomes toast via uploadTick.
        }
      })
    },
    [noteLoggedForDxClear],
  )

  const handleDiscardLog = useCallback(() => {
    void withErrorToast(() => apiDiscardPendingLog(), 'Could not discard QSO').then((s) => {
      if (s) setSnap(s)
    })
  }, [])

  const handleSend = useCallback(
    (text: string) => {
      if (!activePeer) return
      void withErrorToast(
        () => apiSendMessage(activePeer, text),
        'Message could not be sent',
      ).then((s) => s && setSnap(s)) // instant echo — no 300 ms poll wait
    },
    [activePeer],
  )

  // Call CQ / canned broadcast: not directed at a peer — goes to everyone on
  // frequency (the engine prefixes `DE <MYCALL>` and echoes it into the "*" band
  // feed). Surfacing "*" makes that feed visible so the operator sees their own
  // call go out and any replies land in the same pane.
  const surfaceBandFeed = useCallback((s: AppSnapshot | null) => {
    // Only surface the "*" band feed if the broadcast/CQ actually went out (don't yank
    // the operator into the feed on a failure); route the select through withErrorToast
    // so a select failure can't throw an unhandled rejection.
    if (!s) return
    setSnap(s)
    void withErrorToast(() => apiSelectPeer('*'), 'Could not open the band feed').then(
      (s2) => s2 && setSnap(s2),
    )
  }, [])

  const handleBroadcast = useCallback(
    (text: string) => {
      void withErrorToast(() => apiBroadcast(text), 'Could not broadcast').then(surfaceBandFeed)
    },
    [surfaceBandFeed],
  )

  // Call CQ sends a STRUCTURED `CQ <call> <grid>` frame + arms TX (distinct from the
  // free-text broadcast). The backend rejects it if the callsign/grid aren't set, so a
  // CQ never goes out malformed — the error surfaces as a toast.
  const handleCallCq = useCallback(() => {
    void withErrorToast(() => apiCallCq(null), 'Could not call CQ').then(surfaceBandFeed)
  }, [surfaceBandFeed])

  const handleToggleBeacon = useCallback(() => {
    const next = !(snap?.radio.beacon ?? false)
    void withErrorToast(() => apiSetBeacon(next), 'Could not toggle the heartbeat').then((s) => {
      if (s) setSnap(s)
    })
  }, [snap?.radio.beacon])

  const handleSetFrequency = useCallback(
    (dialMhz: number, band: string, mode: string) => {
      void withErrorToast(
        () => apiSetFrequency(dialMhz, band, mode),
        'Could not set frequency',
      ).then((s) => {
        if (s) setSnap(s)
      })
    },
    [],
  )

  const handleSetTxEnabled = useCallback((enabled: boolean) => {
    void withErrorToast(
      () => apiSetTxEnabled(enabled),
      enabled ? 'Could not enable transmit' : 'Could not mute transmit',
    ).then((s) => {
      if (s) setSnap(s)
    })
  }, [])

  const handleSetTxLevel = useCallback((level: number) => {
    void withErrorToast(() => apiSetTxLevel(level), 'Could not set TX level').then((s) => {
      if (s) setSnap(s)
    })
  }, [])

  const handleSetTune = useCallback((on: boolean) => {
    void withErrorToast(() => apiSetTune(on), 'Could not toggle tune').then((s) => {
      if (s) setSnap(s)
    })
  }, [])

  const handleHaltTx = useCallback(() => {
    void withErrorToast(() => apiHaltTx(), 'Could not stop transmit').then((s) => {
      if (s) setSnap(s)
    })
  }, [])

  // WSJT-X Tx-slot click (Tx1–Tx5 buttons / Alt+N): force the row's text as the
  // next transmission to the DX. The backend starts/retargets the QSO + arms TX;
  // applying the returned snapshot makes the Tx panel's "next" dot land at once.
  const handleOverrideTx = useCallback((call: string, grid: string | null, text: string) => {
    // Same own-call guard as handleCall — the engine no-ops on a self-target
    // but returns a normal snapshot, which read as silent success here.
    const me = mycallRef.current.trim().toUpperCase()
    if (me && call.trim().toUpperCase().split('/')[0] === me.split('/')[0]) {
      pushToast(`${call} is your own call`, 'info', 2500)
      return
    }
    void withErrorToast(
      () => apiOverrideNextTx(call, grid, text),
      `Could not queue TX to ${call}`,
    ).then((s) => {
      if (s) setSnap(s)
    })
  }, [])

  const handleSetTxEven = useCallback((even: boolean) => {
    void withErrorToast(() => apiSetTxEven(even), 'Could not set transmit period').then((s) => {
      if (s) setSnap(s)
    })
  }, [])

  const handleSetTxCycleAuto = useCallback((auto: boolean) => {
    void withErrorToast(() => apiSetTxCycleAuto(auto), 'Could not set the cycle mode').then(
      (s) => {
        if (s) setSnap(s)
      },
    )
  }, [])

  const handleSetHoldTxFreq = useCallback((on: boolean) => {
    void withErrorToast(() => apiSetHoldTxFreq(on), 'Could not toggle Hold Tx').then((s) => {
      if (s) setSnap(s)
    })
  }, [])

  // Waterfall click: left-click sets the RX offset (green marker); shift-click
  // sets the TX offset (red marker). TX follows RX unless "Hold Tx" is on.
  const handleTune = useCallback((hz: number, target: 'tx' | 'rx' | 'both') => {
    // Stock WSJT-X gestures (Waterfall dispatches): 'rx' = click, 'tx' = Shift, 'both' = Ctrl.
    const call =
      target === 'rx'
        ? () => apiSetRxOffset(hz)
        : target === 'tx'
          ? () => apiSetTxOffset(hz)
          : () => apiSetTxOffset(hz).then(() => apiSetRxOffset(hz))
    void withErrorToast(call, 'Could not set offset').then((s) => {
      if (s) setSnap(s)
    })
  }, [])

  // QSY from the Needed panel: move the rig to that band's channel and listen.
  const handleQsy = useCallback(
    // `freqMhz` is the spot's EXACT frequency from the spotting network — the source of truth
    // (a DXpedition may be well off the standard FT8/FT4 dial). Fall back to the band's dial
    // only when the spot carries no frequency (e.g. a PSK Reporter reception report).
    (band: string, freqMhz?: number) => {
      const ch = bandPlan.find((c) => c.band === band)
      if (!ch) {
        pushToast(`No channel for ${band} in the band plan`, 'error', 3000)
        return
      }
      const dial = freqMhz ?? ch.dialMhz
      void withErrorToast(
        () => apiSetFrequency(dial, ch.band, ch.mode),
        `Could not QSY to ${band}`,
      ).then((s) => {
        if (s) {
          setSnap(s)
          pushToast(`QSY ${dial.toFixed(3)} MHz — listening`, 'success', 2500)
        }
      })
    },
    [bandPlan],
  )

  // Click-to-work ANY needed spot (CW / Phone / Digital): N1MM-style single click that
  // changes the band, mode, AND frequency to exactly the spot's — in ONE atomic backend call
  // (`workSpot`) so the rig can never end up in the new mode at the old dial (no wrong-mode
  // flash) and the UI never sees a half-applied mode/freq state. Then open the matching
  // cockpit and — for CW/Phone — hand it the callsign to prefill the log. A need with no
  // Point the antenna rotator at a needed call (great-circle bearing from your grid).
  const handlePointAntenna = useCallback(async (call: string) => {
    try {
      const bearing = await pointRotatorAtCall(call)
      pushToast(`↗ Pointing antenna to ${Math.round(bearing)}° (${call})`, 'success', 3000)
    } catch (e) {
      pushToast(typeof e === 'string' ? e : `Couldn't point the antenna at ${call}`, 'error', 4000)
    }
  }, [])

  // resolvable frequency at all falls back to a plain band QSY.
  const handleWorkNeeded = useCallback(
    (alert: NeedAlert) => {
      const t = workTarget(alert, bandPlan)
      if (!t) {
        handleQsy(alert.band)
        return
      }
      // The Needed board now lists ALL modes (W1), but the CW/Phone cockpits are opt-in
      // features. If the target cockpit is disabled, don't navigate into a hidden view
      // (that dumped the operator on the landing page) — just QSY the rig to the spot.
      if ((t.view === 'cw' && !cwEnabled) || (t.view === 'phone' && !phoneEnabled)) {
        handleQsy(alert.band)
        return
      }
      // 'operate' is the digital cockpit, so its operating mode is 'digital'.
      const opMode: 'digital' | 'phone' | 'cw' = t.view === 'operate' ? 'digital' : t.view
      void withErrorToast(
        () => workSpot(opMode, t.freqMhz, t.band, t.call),
        `Could not work ${t.call} — check CAT`,
      ).then((s) => {
        // On failure DON'T navigate or poison the guard ref — the backend made no change
        // (atomic), so the view-effect can still apply the mode on a later nav.
        if (!s) return
        setSnap(s)
        // Keep the rig-mode effect's guard in sync so it doesn't re-fire on the nav.
        lastOpModeRef.current = opMode
        // CW/Phone cockpits consume a prefill; the digital cockpit auto-sequences on a decode
        // double-click, so it gets no prefill — just the QSY + DATA-U.
        if (t.view !== 'operate') {
          setPendingWork({ call: t.call, view: t.view, ts: Date.now() })
        }
        setView(t.view)
        pushToast(`▶ ${t.call} — ${alert.mode} ${t.band}, ready to log`, 'success', 4000)
      })
    },
    [bandPlan, handleQsy, cwEnabled, phoneEnabled],
  )

  // Work a spot double-clicked on the MAP — the same atomic path as the Needed
  // board (workSpot → rig jumps band+mode+freq, cockpit opens). The source-reported
  // mode routes the cockpit: CW→CW, SSB/FM→Phone, FT8/unknown→Digital.
  const handleWorkMapSpot = useCallback(
    (t: { call: string; band: string; mode: string | null; freqMhz: number | null }) => {
      handleWorkNeeded({
        call: t.call,
        entity: '',
        band: t.band,
        zone: 0,
        tags: [],
        priority: 0,
        headline: '',
        mode: modeClassOf(t.mode),
        freqMhz: t.freqMhz,
      })
    },
    [handleWorkNeeded],
  )
  // The chase toast's "Work" action — assigned via ref because the prop poller
  // (defined above, deps []) closes over its startup scope. Routes by the
  // expedition's announced modes like every other DXpedition work path (a
  // CW-only op must open the CW cockpit at the CW activity freq, not FT8).
  workDxpedRef.current = (c: WorkableCard) =>
    handleWorkMapSpot({ call: c.call, band: c.band, mode: dxpedWorkMode(c.modes), freqMhz: null })

  // Hunt a POTA/SOTA activator: tag the next QSO with the park/summit reference AND
  // QSY to the spot's exact frequency — the same atomic workSpot path as handleWorkNeeded.
  // PotaSotaView calls setHuntTarget itself (and hands us the fresh snap via onSnap),
  // then calls this handler which only needs to do the QSY + navigation + toast.
  const handleHuntSpot = useCallback(
    (arg: OtaSpotClickArg) => {
      // Build a minimal NeedAlert-shaped object so we can reuse handleWorkNeeded's
      // existing workSpot → cockpit-open → pendingWork path exactly.
      handleWorkNeeded({
        call: arg.call,
        entity: '',
        band: arg.band,
        zone: 0,
        tags: [],
        priority: 0,
        headline: '',
        mode: arg.modeClass,
        freqMhz: arg.freqMhz,
      })
      // (handleWorkNeeded toasts the QSY itself — one toast per action.)
    },
    [handleWorkNeeded],
  )

  // Work a raw spot from the Spots panel — synthesize a minimal NeedAlert so we reuse
  // handleWorkNeeded's workSpot → cockpit-open path (QSY to the spot's exact freq + mode).
  const handleWorkSpot = useCallback(
    (s: SpotRow) => {
      handleWorkNeeded({
        call: s.call,
        entity: s.entity,
        band: s.band,
        zone: s.zone,
        tags: [],
        priority: 0,
        headline: '',
        mode: s.mode,
        freqMhz: s.freqMhz,
      })
    },
    [handleWorkNeeded],
  )

  // Stop a QSO recording from anywhere (the global REC badge in the TopBar), so an active
  // recording started in the Phone cockpit can be stopped without navigating back.
  const handleStopRecording = useCallback(() => {
    void stopQsoRecording()
      .then((s) => s && setSnap(s))
      .catch(() => {})
  }, [])

  const handleSetMode = useCallback((mode: ModeRequest) => {
    void withErrorToast(() => apiSetMode(mode), 'Could not switch mode').then((s) => {
      if (s) setSnap(s)
    })
  }, [])

  const handleQsoResend = useCallback(() => {
    void withErrorToast(() => apiQsoResend(), 'Could not resend').then((s) => {
      if (s) setSnap(s)
    })
  }, [])

  const handleQsoFreetext = useCallback((text: string) => {
    void withErrorToast(() => apiQsoFreetext(text), 'Could not send free text').then((s) => {
      if (s) setSnap(s)
    })
  }, [])

  const handleLogCurrent = useCallback(() => {
    void withErrorToast(() => apiLogCurrentQso(), 'Could not log QSO').then((s) => {
      if (s) {
        setSnap(s)
        pushToast('Logged QSO', 'success', 2500)
        noteLoggedForDxClear()
        // QRZ/ClubLog/eQSL auto-upload happens in the BACKEND log funnel now
        // (every log path, auto-log included); outcomes toast via uploadTick.
      }
    })
  }, [noteLoggedForDxClear])

  // Selecting a view from the nav. QSO / Field Day also request the backend mode
  // (defaulting to the "run" / "chat" role); Settings are pure UI
  // screens that leave the operating mode unchanged.
  const handleView = useCallback(
    (next: View) => {
      setView(next)
      // Passive-first: entering QSO / Field Day starts in Search-&-Pounce
      // (listen + answer), never auto-calling CQ. The operator hits "Call CQ" /
      // "Running" in the panel to start transmitting.
      if (next === 'chat') handleSetMode('chat')
      else if (next === 'fieldDay') handleSetMode('fieldday-sp')
    },
    [handleSetMode],
  )

  const handleTier = useCallback((t: Tier) => {
    void withErrorToast(() => apiSetTier(t), 'Could not change tier').then((s) => {
      if (s) setSnap(s)
    })
  }, [])

  // Pick a Digital sub-mode from the rail. Tempo → the FT1/DX1 free-text cockpit
  // (reuse the workspace bind). Digital → the weak-signal cockpit, PRESERVING the
  // last FT8/FT4 tier (the top bar's pills own that choice now — the rail button
  // must not yank an FT4 operator back to FT8): bind the dx workspace (tier + QSO
  // mode) THEN re-assert the tier, sequentially, so set_area's default-FT8 can't
  // race past it.
  const handleDigitalMode = useCallback(
    (m: DigitalMode) => {
      if (m === 'tempo') {
        handleWorkspace('msg')
        return
      }
      const wantTier: Tier = tierRef.current === 'FT4' ? 'FT4' : 'FT8'
      setArea('dx')
      try {
        localStorage.setItem('nexus.workspace', 'dx')
      } catch {
        /* ignore */
      }
      setView('operate')
      // The codec tier (FT8/FT4) is INDEPENDENT of the rig's CAT mode — switching tiers does
      // NOT command the Yaesu into DATA-U. Assert the digital rig mode explicitly here:
      // clicking a Digital sub-mode while already on the Operate screen doesn't change `view`,
      // so the rig-mode view-effect never fires and the rig would stay in whatever (SSB/CW)
      // mode it was last left in. follow_freq=false keeps the current digital frequency.
      lastOpModeRef.current = 'digital'
      void setOperatingMode('digital', false)
        .then((s) => s && setSnap(s))
        .catch(() => {})
      // Bind the dx workspace, THEN set the exact tier — each wrapped so a backend
      // failure surfaces a toast (matching handleTier) instead of failing silently
      // or leaving an unhandled rejection.
      void withErrorToast(() => apiSetArea('dx'), 'Could not switch to Digital')
        .then((s) => {
          if (s) setSnap(s)
          return withErrorToast(() => apiSetTier(wantTier), 'Could not change tier')
        })
        .then((s) => {
          if (s) setSnap(s)
        })
    },
    [handleWorkspace],
  )

  const handleSourceChange = useCallback((k: SourceKind) => {
    // Companion bind can fail (port busy) → withErrorToast surfaces it and the
    // backend stays on the previous source.
    void withErrorToast(() => apiSetSource(k), 'Could not switch signal source').then((s) => {
      if (s) {
        setSnap(s)
        // Confirm the switch even when no decodes follow (e.g. Companion idle).
        pushToast(
          k === 'companion'
            ? `Source: ${s.radio.sourceLabel} — listening for WSJT-X/JTDX/MSHV on :2237`
            : `Source: ${s.radio.sourceLabel}`,
          'success',
          3500,
        )
      }
    })
  }, [])

  const handleSettingsSaved = useCallback(() => {
    getSnapshot().then(setSnap).catch(() => {})
    reloadSettings()
  }, [reloadSettings])

  const handleDismissOnboarding = useCallback(() => {
    localStorage.setItem(ONBOARD_KEY, '1')
    setOnboardDismissed(true)
  }, [])

  const handleWizardApply = useCallback(
    (ids: ProfileId[], landing: View, modes: FeatureId[], license: string) => {
      // Goal profiles + the chosen operating modes (CW/Phone) force-enabled on top.
      features.applyProfiles(ids, modes)
      // Persist the declared license class (drives the TX-privilege lockout).
      void setLicenseClass(license)
        .then((s) => s && setSnap(s))
        .catch(() => {})
      setView(landing)
      markWizardSeen()
      setShowWizard(false)
    },
    [features.applyProfiles],
  )

  const handleWizardSkip = useCallback(() => {
    markWizardSeen()
    setShowWizard(false)
  }, [])

  if (!snap) {
    return (
      <div className="app loading">
        <span>Connecting to Nexus…</span>
      </div>
    )
  }

  const macros = settings?.macros ?? DEFAULT_MACROS

  const activeConversation =
    snap.conversations.find((c) => c.peer === activePeer) ?? null

  // displayed tier is the authoritative link tier from the snapshot
  const tier = snap.link.tier

  // Defense in depth: if the current view's feature got disabled (e.g. toggled
  // off in Settings while viewing it), fall back to the profile's landing view.
  // The nav already hides disabled sections; this guards a stale selection.
  const effectiveView: View = features.isOn(view as FeatureId) ? view : features.landing

  // First-run nudge: callsign unset / still the placeholder, and not dismissed.
  const needsOnboarding =
    !onboardDismissed &&
    effectiveView !== 'settings' &&
    snap.mycall.trim() === '' // fresh install (the default callsign is empty)

  // The Tempo (chat) roster represents who's on the TEMPO protocol — so it shows only
  // stations last heard on FT1, not the FT8/FT4 stations that share the engine's single
  // roster. Every other view (Operate, Field Day) shows the full roster.
  const rosterStations =
    effectiveView === 'chat' ? snap.stations.filter((s) => s.tier === 'FT1') : snap.stations
  const stationsPanel = (
    <StationList
      stations={rosterStations}
      myGrid={snap.mygrid}
      currentSlot={snap.radio.slot}
      activePeer={activePeer}
      unreadByPeer={unreadByPeer}
      needByCall={needByCall}
      onSelect={handleSelect}
      onCall={handleCall}
      conversations={snap.conversations}
      onArchive={handleArchive}
      bandActive={activePeer === '*'}
      bandUnread={bandUnread}
      onSelectBand={() => handleSelect('*')}
    />
  )

  // Pane resize: dragging a splitter writes the rail-width CSS var directly each
  // frame (no React re-render), then commits (clamp + persist) on pointer-up.
  // One Pointer-Events path covers mouse, touch, and pen.
  const startResize =
    (side: 'left' | 'right') => (e: React.PointerEvent<HTMLDivElement>) => {
      const el = layoutRef.current
      if (!el) return
      e.preventDefault()
      ;(e.target as HTMLElement).setPointerCapture(e.pointerId)
      const rect = el.getBoundingClientRect()
      const GAP = 12 // .layout padding; keeps the rail edge under the pointer
      const root = document.documentElement.style
      document.body.classList.add('resizing')
      const widthFor = (clientX: number) =>
        side === 'right' ? rect.right - GAP - clientX : clientX - rect.left - GAP
      const move = (ev: PointerEvent) => {
        const w = widthFor(ev.clientX)
        root.setProperty(side === 'right' ? '--right-rail-w' : '--left-rail-w', `${
          side === 'right' ? clampRight(w) : clampLeft(w)
        }px`)
      }
      const up = (ev: PointerEvent) => {
        if (side === 'right') commitRight(widthFor(ev.clientX))
        else commitLeft(widthFor(ev.clientX))
        window.removeEventListener('pointermove', move)
        window.removeEventListener('pointerup', up)
        document.body.classList.remove('resizing')
      }
      window.addEventListener('pointermove', move)
      window.addEventListener('pointerup', up)
    }

  const waterfallRail = (
    <aside className="right-rail panel">
      <Waterfall
        transmitting={snap.radio.transmitting}
        rxOffsetHz={snap.radio.rxOffsetHz}
        txOffsetHz={snap.radio.txOffsetHz}
        theme={theme}
        onTune={handleTune}
      />
      <OperateDecodes
        decodes={snap.recentDecodes}
        slot={snap.radio.slot}
        rxOffsetHz={snap.radio.rxOffsetHz}
        band={snap.radio.band}
        tier={snap.link.tier}
        harqRescues={snap.harqRescues}
        onCall={handleCall}
        needAlertsByCall={needAlertsByCall}
        compact
        title="Band Activity — heard on the band"
      />
      <LinkPill link={snap.link} radio={snap.radio} />
    </aside>
  )

  // Three-pane workspace: stations | center | waterfall, with drag splitters
  // between each. CSS (keyed on `data-layout`) places the waterfall on the right
  // (default) or as a full-width strip on top — same JSX, no remount.
  const threePane = (center: JSX.Element) => (
    <main className="layout" data-three-pane ref={layoutRef}>
      <div className="grid-stations">{stationsPanel}</div>
      <div
        className="pane-splitter left"
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize stations panel (double-click to reset)"
        onPointerDown={startResize('left')}
        onDoubleClick={resetWidths}
      />
      <div className="grid-center">{center}</div>
      <div
        className="pane-splitter right"
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize waterfall pane (double-click to reset)"
        onPointerDown={startResize('right')}
        onDoubleClick={resetWidths}
      />
      <div className="grid-waterfall">{waterfallRail}</div>
    </main>
  )

  let workspace: JSX.Element | null
  switch (effectiveView) {
    case 'fieldDay':
      workspace = threePane(
        <FieldDayView fieldDay={snap.fieldDay} onSetMode={handleSetMode} />,
      )
      break
    case 'logbook':
      workspace = (
        <main className="layout single">
          <Logbook
            defaultBand={snap.radio.band}
            defaultFreqMhz={snap.radio.dialMhz}
            defaultMode={snap.link.tier}
          />
        </main>
      )
      break
    case 'needed':
      workspace = (
        <NeededPanel
          // FULL un-gated list: the board's own per-mode toggles decide what shows, so a
          // disabled CW/Phone *feature* no longer hides those needs here (the operator
          // controls mode visibility in the Needed filter bar instead).
          alerts={needAlerts}
          bandPlan={bandPlan}
          selectedCall={activePeer}
          onQsy={(a) => handleQsy(a.band, a.freqMhz ?? undefined)}
          onSelect={handleSelect}
          onWork={handleWorkNeeded}
          onPoint={settings?.rotatorHost?.trim() ? handlePointAntenna : undefined}
          onPopOut={() => void openPanelWindow('needed')}
          phoneSource={
            feedHealth
              ? {
                  status: feedHealth.phoneCluster,
                  host: feedHealth.phoneClusterHost,
                  spotsSeen: feedHealth.phoneSpotsSeen,
                }
              : null
          }
        />
      )
      break
    case 'spots':
      workspace = (
        <SpotsPanel
          spots={allSpots}
          bandPlan={bandPlan}
          selectedCall={activePeer}
          onSelect={handleSelect}
          onWork={handleWorkSpot}
        />
      )
      break
    case 'awards':
      // Awards + Journey combined: one section, tabbed (Journey + Official Awards).
      workspace = <AwardsJourney showGamification={features.isOn('gamification')} />
      break
    case 'cw':
      workspace = (
        <CwCockpit
          pitchHz={settings?.cwPitchHz ?? 600}
          snap={snap}
          theme={theme}
          pendingWork={pendingWork?.view === 'cw' ? pendingWork : null}
          onConsumeWork={() => setPendingWork(null)}
          onSnap={setSnap}
          fieldDay={snap.fieldDay}
        />
      )
      break
    case 'phone':
      workspace = (
        <PhoneCockpit
          snap={snap}
          theme={theme}
          pendingWork={pendingWork?.view === 'phone' ? pendingWork : null}
          onConsumeWork={() => setPendingWork(null)}
          onSnap={setSnap}
          fieldDay={snap.fieldDay}
          phoneMode={settings?.phoneMode}
        />
      )
      break
    case 'pota':
      workspace = (
        <main className="layout single">
          <PotaSotaView
            snap={snap}
            onHunt={handleHuntSpot}
            onSnap={setSnap}
          />
        </main>
      )
      break
    case 'settings':
      workspace = (
        <main className="layout single">
          <SettingsPanel
            onSaved={handleSettingsSaved}
            radio={snap.radio}
            layout={wfLayout}
            onLayoutChange={setWfLayout}
            scale={scale}
            onScaleChange={setScale}
            onResetLayout={resetWidths}
            features={features}
            onRerunWizard={() => setShowWizard(true)}
          />
        </main>
      )
      break
    case 'operate':
      // The Operate cockpit is NOT rendered here — it stays permanently mounted in
      // a persistent host below (so its waterfall + Band Activity keep accumulating
      // in the background across navigation). This case renders nothing in the slot.
      workspace = null
      break
    case 'connect':
      workspace = (
        <ConnectView
          myGrid={settings?.mygrid ?? ''}
          theme={theme}
          stations={snap?.stations ?? []}
          prop={prop}
          selectedCall={activePeer}
          onSelectCall={handleMapSelect}
          needByCall={needByCall}
          onWorkSpot={handleWorkMapSpot}
          needAlerts={visibleAlerts}
          onPoint={settings?.rotatorHost?.trim() ? handlePointAntenna : undefined}
          onPopOut={() => void openPanelWindow('connect')}
        />
      )
      break
    case 'dxped':
      workspace = (
        <main className="layout single">
          <DxpeditionsView
            snap={prop}
            onWorkSpot={handleWorkMapSpot}
            onShowOnMap={(call) => {
              // Hand off to Connect with the expedition selected on the map.
              handleMapSelect(call)
              setView('connect')
            }}
            onPopOut={() => void openPanelWindow('dxped')}
          />
        </main>
      )
      break
    case 'chat':
    default:
      workspace = (
        <>
          {threePane(
            <Conversation
              conversation={activeConversation}
              peer={activePeer}
              radio={snap.radio}
              mode={snap.mode}
              fieldDay={snap.fieldDay}
              macros={macros}
              onSend={handleSend}
              onBroadcast={handleBroadcast}
              onCallCq={handleCallCq}
              beaconOn={snap.radio.beacon ?? false}
              onToggleBeacon={handleToggleBeacon}
              mycall={snap.mycall}
              mygrid={snap.mygrid}
              // Roam (coordinated QSY) lives INSIDE Tempo now: the chip toggles
              // it, the gear opens the full panel (was its own rail section).
              roamEnabled={snap.qsy?.enabled ?? false}
              roamStatus={snap.qsy?.enabled ? (snap.qsy.paused ? 'paused' : (snap.qsy.current ?? 'on')) : undefined}
              onToggleRoam={() =>
                void withErrorToast(
                  () => apiQsySetEnabled(!(snap.qsy?.enabled ?? false)),
                  'Could not toggle Roam',
                ).then((s) => s && setSnap(s))
              }
              onRoamSettings={() => setRoamOpen(true)}
            />,
          )}
          {roamOpen && (
            <div className="roam-modal" role="dialog" aria-label="Roam settings">
              <div className="roam-modal-body">
                <button
                  type="button"
                  className="roam-modal-close"
                  onClick={() => setRoamOpen(false)}
                  aria-label="Close Roam settings"
                >
                  ✕
                </button>
                <RoamPanel
                  qsy={snap.qsy ?? null}
                  channels={settings?.qsySet ?? []}
                  cadence={settings?.qsyCadence ?? 6}
                  bandPlan={bandPlan}
                  activePeer={activePeer}
                  onSnap={setSnap}
                  onReloadSettings={reloadSettings}
                />
              </div>
            </div>
          )}
        </>
      )
      break
  }

  return (
    <div className="app">
      <TopBar
        mycall={snap.mycall}
        mygrid={snap.mygrid}
        radio={snap.radio}
        link={snap.link}
        bandPlan={bandPlan}
        onSetFrequency={handleSetFrequency}
        onSetTxEnabled={handleSetTxEnabled}
        onSetTune={handleSetTune}
        onHaltTx={handleHaltTx}
        onSetTxEven={handleSetTxEven}
            onSetTxCycleAuto={handleSetTxCycleAuto}
        onSetHoldTxFreq={handleSetHoldTxFreq}
        onStopRecording={handleStopRecording}
        wfLayout={wfLayout}
        onWfLayoutChange={setWfLayout}
        tier={tier}
        onTierChange={handleTier}
        theme={theme}
        onThemeChange={setTheme}
      />

      {needsOnboarding && (
        <OnboardingBanner
          onOpenSettings={() => handleView('settings')}
          onDismiss={handleDismissOnboarding}
        />
      )}

      {reveal.pending && (
        <RevealNudge
          feature={reveal.pending.feature}
          achievement={reveal.pending.achievement}
          onEnable={reveal.enable}
          onDismiss={reveal.dismiss}
        />
      )}

      <NowBar
        snap={snap}
        prop={prop}
        feedHealth={feedHealth}
        connectEnabled={features.isOn('connect')}
        dxpedEnabled={features.isOn('dxped')}
        onNavigate={handleView}
      />

      <div className="shell">
        <ModeNav
          view={effectiveView}
          mode={snap.mode}
          enabled={features.enabled}
          onSelect={handleView}
          tier={tier}
          onDigitalMode={handleDigitalMode}
        />
        {/* Operate cockpit lives here PERMANENTLY (mounted once, hidden when you're
            on another section) so the waterfall + Band Activity keep decoding and
            accumulating in the background — navigate away and back and your decodes
            are exactly where you left them, plus everything heard while away. The
            host is display:contents when shown (so the inner <main> flexes exactly
            as before) and display:none when hidden. */}
        <div className="operate-host" hidden={effectiveView !== 'operate'}>
          <OperateCockpit
            snap={snap}
            theme={theme}
            tier={tier}
            onTierChange={handleTier}
            onSourceChange={handleSourceChange}
            onTune={handleTune}
            onCall={handleCall}
            onSetTxLevel={handleSetTxLevel}
            onSetMode={handleSetMode}
            onSetTxEven={handleSetTxEven}
            onSetTxCycleAuto={handleSetTxCycleAuto}
            onResend={handleQsoResend}
            onFreetext={handleQsoFreetext}
            onLog={handleLogCurrent}
            onOverrideTx={handleOverrideTx}
            onHaltTx={handleHaltTx}
            dxClearTick={dxClearTick}
            onSnap={setSnap}
            preferRrr={settings?.preferRrr ?? false}
            qsoMacros={macros.qso}
            roster={stationsPanel}
            needByCall={needByCall}
            needAlertsByCall={needAlertsByCall}
            selectedCall={activePeer}
            onSelect={handleSelect}
            layoutMode={operateLayout}
            onLayoutMode={handleOperateLayout}
            onPopOut={() => void openPanelWindow('operate')}
            active={effectiveView === 'operate'}
          />
        </div>
        {workspace}
      </div>

      <Toasts />

      {showWizard && <SetupWizard onApply={handleWizardApply} onSkip={handleWizardSkip} />}

      {snap.pendingLog && (
        <LogConfirm
          record={snap.pendingLog}
          onConfirm={handleConfirmLog}
          onDiscard={handleDiscardLog}
        />
      )}
    </div>
  )
}
