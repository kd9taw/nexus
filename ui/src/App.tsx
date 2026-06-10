import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { AppSnapshot, BandChannel, LoggedQso, ModeRequest, Settings, SourceKind, Tier } from './types'
import {
  broadcast as apiBroadcast,
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
  peerIsTyping,
  selectPeer as apiSelectPeer,
  sendMessage as apiSendMessage,
  setFrequency as apiSetFrequency,
  setMode as apiSetMode,
  setTier as apiSetTier,
  setSource as apiSetSource,
  setTxEnabled as apiSetTxEnabled,
  setTxLevel as apiSetTxLevel,
  setTune as apiSetTune,
  haltTx as apiHaltTx,
  setTxEven as apiSetTxEven,
  setRxOffset as apiSetRxOffset,
  setTxOffset as apiSetTxOffset,
  setHoldTxFreq as apiSetHoldTxFreq,
  subscribeSnapshot,
  isTauri,
} from './api'
import { withErrorToast, pushToast } from './toast'
import { processDecodes } from './alerts'
import { useTheme } from './useTheme'
import { useLayout } from './useLayout'
import { useScale } from './useScale'
import { useDensity } from './useDensity'
import { useMotion } from './useMotion'
import { useAchievements } from './useAchievements'
import { useJourneyUnlocks } from './useJourneyUnlocks'
import { useFeatures } from './useFeatures'
import { useReveals } from './useReveals'
import { sectionFeatures, featureById, type FeatureId } from './features/registry'
import { visibleNeeds, workTarget, modeClassOf } from './features/needs'
import { autoPushQso } from './features/autopush'
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
import { PotaSotaView } from './components/PotaSotaView'
import { DxpeditionsView } from './components/DxpeditionsView'
import { ConnectView } from './components/ConnectView'
import {
  getLog,
  getPropagation,
  getFeedHealth,
  getNeedAlerts,
  setOperatingMode,
  workSpot,
  setLicenseClass,
  stopQsoRecording,
} from './api'
import { setStatus } from './status'
import type { PropagationSnapshot, FeedHealth, NeedTag, NeedAlert } from './types'
import { NeededPanel } from './components/NeededPanel'
import { LogConfirm } from './components/LogConfirm'
import { FieldDayView } from './components/FieldDayView'
import { BandFeed } from './components/BandFeed'
import { DecodeFeed } from './components/DecodeFeed'
import { Logbook } from './components/Logbook'
import { LogView } from './components/LogView'
import { RoamPanel } from './components/RoamPanel'
import { SettingsPanel } from './components/SettingsPanel'
import { Toasts } from './components/Toasts'
import { OnboardingBanner } from './components/OnboardingBanner'
import { RevealNudge } from './components/RevealNudge'
import { SetupWizard } from './components/SetupWizard'
import type { ProfileId } from './features/profiles'
import { DemoBanner } from './components/DemoBanner'

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
// Synthetic peer key for the open band-activity / broadcast feed.
const BROADCAST_PEER = '*'

// Macros fall back to these until settings load (keeps chips populated).
const DEFAULT_MACROS: Settings['macros'] = {
  chat: ['73', 'QSL', 'Name?', 'QTH?', 'CQ'],
  qso: ['R-09', 'RRR', 'RR73', '73'],
  band: ['CQ CQ', 'QRZ?', 'Net check-in', '73 to all'],
}

export default function App() {
  const [theme, setTheme] = useTheme()
  const [wfLayout, setWfLayout] = useLayout()
  const [scale, setScale] = useScale()
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

  // Per-(band,mode) last-alert time so a band coming alive toasts once, not every
  // poll (defence in depth — the backend tracker already flags `isNew` once).
  const openingAlertRef = useRef<Map<string, number>>(new Map())
  useEffect(() => {
    let live = true
    const OPENING_ALERT_COOLDOWN_MS = 10 * 60_000
    const load = () =>
      getPropagation()
        .then((p) => {
          if (!live) return
          setProp(p)
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
          if (p.source === 'demo') {
            setStatus('prop', {
              tier: 'warning',
              message: 'Prop: demo data',
              detail: 'Propagation is showing the offline demo scene — set your callsign in Settings for live data.',
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
  // Gate CW/Phone needs by the operator's enabled modes — the backend emits voice/CW
  // needs unconditionally, visibility is the frontend's call. A pure-digital op's board,
  // roster colouring, and map highlight all derive from THIS gated set, so they stay
  // exactly as before the feature.
  const cwEnabled = features.isOn('cw')
  const phoneEnabled = features.isOn('phone')
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

  // poll the (mock) typing state so the indicator re-renders smoothly
  useEffect(() => {
    const id = window.setInterval(() => setTypingTick((t) => t + 1), 400)
    return () => window.clearInterval(id)
  }, [])

  // Fire decode alerts (beep + toast) whenever the decode feed changes, gated
  // by the user's alert settings. processDecodes dedups internally.
  useEffect(() => {
    if (!snap || !settings) return
    processDecodes(snap.recentDecodes, settings)
  }, [snap, settings])

  const activePeer = snap?.activePeer ?? null

  // mark the active conversation as read whenever it updates
  useEffect(() => {
    if (!snap || !activePeer) return
    const conv = snap.conversations.find((c) => c.peer === activePeer)
    if (conv) readCounts.current[activePeer] = conv.messages.length
  }, [snap, activePeer])

  const unreadByPeer = useMemo(() => {
    const out: Record<string, number> = {}
    if (!snap) return out
    for (const c of snap.conversations) {
      // the "*" band feed is not a roster peer; don't surface unread badges for it
      if (c.peer === BROADCAST_PEER) continue
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

  const handleSelect = useCallback((call: string) => {
    void withErrorToast(() => apiSelectPeer(call), 'Could not select station').then(
      (s) => s && setSnap(s),
    )
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
          // The Settings auto-upload toggles apply to EVERY log path — this
          // (prompt-to-log) used to silently skip QRZ/ClubLog/eQSL.
          void autoPushQso(record, {
            qrz: settings?.qrzLogbookUpload ?? false,
            clublog: settings?.clublogUpload ?? false,
            eqsl: settings?.eqslUpload ?? false,
          })
        }
      })
    },
    [settings, noteLoggedForDxClear],
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

  const handleBroadcast = useCallback((text: string) => {
    void withErrorToast(() => apiBroadcast(text), 'Broadcast could not be sent').then((s) => {
      if (s) setSnap(s)
    })
  }, [])

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
    (band: string) => {
      const ch = bandPlan.find((c) => c.band === band)
      if (!ch) {
        pushToast(`No channel for ${band} in the band plan`, 'error', 3000)
        return
      }
      void withErrorToast(
        () => apiSetFrequency(ch.dialMhz, ch.band, ch.mode),
        `Could not QSY to ${band}`,
      ).then((s) => {
        if (s) {
          setSnap(s)
          pushToast(`QSY ${band} — listening`, 'success', 2500)
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
  // resolvable frequency at all falls back to a plain band QSY.
  const handleWorkNeeded = useCallback(
    (alert: NeedAlert) => {
      const t = workTarget(alert, bandPlan)
      if (!t) {
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
    [bandPlan, handleQsy],
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
    void withErrorToast(() => apiLogCurrentQso(), 'Could not log QSO').then(async (s) => {
      if (s) {
        setSnap(s)
        pushToast('Logged QSO', 'success', 2500)
        noteLoggedForDxClear()
        // Auto-upload the JUST-logged QSO (the newest log row) — the cockpit
        // path used to silently skip QRZ/ClubLog/eQSL regardless of Settings.
        const anyPush =
          (settings?.qrzLogbookUpload || settings?.clublogUpload || settings?.eqslUpload) ?? false
        if (anyPush) {
          try {
            const log = await getLog()
            const newest = log[log.length - 1]
            if (newest) {
              void autoPushQso(newest, {
                qrz: settings?.qrzLogbookUpload ?? false,
                clublog: settings?.clublogUpload ?? false,
                eqsl: settings?.eqslUpload ?? false,
              })
            }
          } catch {
            /* log fetch failed — local log is intact; push next time */
          }
        }
      }
    })
  }, [settings, noteLoggedForDxClear])

  // Selecting a view from the nav. QSO / Field Day also request the backend mode
  // (defaulting to the "run" / "chat" role); Band / Log / Settings are pure UI
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
  // (reuse the workspace bind). FT8/FT4 → the weak-signal cockpit on that tier: bind
  // the dx workspace (tier + QSO mode) THEN set the exact tier, sequentially, so
  // set_area's default-FT8 can't race past a requested FT4.
  const handleDigitalMode = useCallback(
    (m: DigitalMode) => {
      if (m === 'tempo') {
        handleWorkspace('msg')
        return
      }
      const wantTier: Tier = m === 'ft8' ? 'FT8' : 'FT4'
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
  const broadcastConversation =
    snap.conversations.find((c) => c.peer === BROADCAST_PEER) ?? null
  const peerTyping = activePeer ? peerIsTyping(activePeer) : false
  // typingTick keeps this expression re-evaluated
  void typingTick

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

  const stationsPanel = (
    <StationList
      stations={snap.stations}
      myGrid={snap.mygrid}
      currentSlot={snap.radio.slot}
      activePeer={activePeer}
      unreadByPeer={unreadByPeer}
      needByCall={needByCall}
      onSelect={handleSelect}
      onCall={handleCall}
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
      <DecodeFeed decodes={snap.recentDecodes} harqRescues={snap.harqRescues} onCall={handleCall} />
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
    case 'band':
      workspace = threePane(
        <BandFeed
          conversation={broadcastConversation}
          mycall={snap.mycall}
          macros={macros}
          onBroadcast={handleBroadcast}
        />,
      )
      break
    case 'logbook':
      workspace = (
        <main className="layout single">
          <Logbook
            defaultBand={snap.radio.band}
            defaultFreqMhz={snap.radio.dialMhz}
            defaultMode={snap.link.tier}
            qrzUpload={settings?.qrzLogbookUpload ?? false}
            clublogUpload={settings?.clublogUpload ?? false}
            eqslUpload={settings?.eqslUpload ?? false}
          />
        </main>
      )
      break
    case 'needed':
      workspace = (
        <NeededPanel
          alerts={visibleAlerts}
          bandPlan={bandPlan}
          selectedCall={activePeer}
          onQsy={handleQsy}
          onSelect={handleSelect}
          onWork={handleWorkNeeded}
          onPopOut={() => void openPanelWindow('needed')}
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
        />
      )
      break
    case 'pota':
      workspace = (
        <main className="layout single">
          <PotaSotaView />
        </main>
      )
      break
    case 'log':
      workspace = (
        <main className="layout single">
          <LogView snap={snap} />
        </main>
      )
      break
    case 'roam':
      workspace = (
        <main className="layout single">
          <RoamPanel
            qsy={snap.qsy ?? null}
            channels={settings?.qsySet ?? []}
            cadence={settings?.qsyCadence ?? 6}
            bandPlan={bandPlan}
            activePeer={activePeer}
            onSnap={setSnap}
            onReloadSettings={reloadSettings}
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
          />
        </main>
      )
      break
    case 'chat':
    default:
      workspace = threePane(
        <Conversation
          conversation={activeConversation}
          peer={activePeer}
          radio={snap.radio}
          mode={snap.mode}
          fieldDay={snap.fieldDay}
          macros={macros}
          peerTyping={peerTyping}
          onSend={handleSend}
        />,
      )
      break
  }

  return (
    <div className="app">
      {!isTauri() && <DemoBanner />}
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
            selectedCall={activePeer}
            onSelect={handleSelect}
            layoutMode={operateLayout}
            onLayoutMode={handleOperateLayout}
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
