import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { AppSnapshot, BandChannel, LoggedQso, ModeRequest, Settings, SourceKind, Tier } from './types'
import {
  broadcast as apiBroadcast,
  callStation as apiCallStation,
  setArea as apiSetArea,
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
import { useFeatures } from './useFeatures'
import { useReveals } from './useReveals'
import { sectionFeatures, type FeatureId } from './features/registry'
import { usePaneWidths, clampLeft, clampRight } from './usePaneWidths'
import { TopBar } from './components/TopBar'
import { StationList } from './components/StationList'
import { Conversation } from './components/Conversation'
import { Waterfall } from './components/Waterfall'
import { LinkPill } from './components/LinkPill'
import { ModeNav, type View } from './components/ModeNav'
import { OperateCockpit } from './components/OperateCockpit'
import { NowBar } from './components/NowBar'
import { AwardsView } from './components/AwardsView'
import { PotaSotaView } from './components/PotaSotaView'
import { PropagationView } from './components/PropagationView'
import { MapView } from './components/MapView'
import { ConnectView } from './components/ConnectView'
import { getPropagation, getFeedHealth, getNeedAlerts } from './api'
import { setStatus } from './status'
import type { PropagationSnapshot, FeedHealth, NeedTag } from './types'
import { QsoPanel } from './components/QsoPanel'
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

// Placeholder identity shipped by the mock/default config. Until the operator
// sets a real callsign we nudge them toward Settings.
const PLACEHOLDER_CALL = 'KD9TAW'
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
  const reveal = useReveals(features)
  // First-run setup wizard (goal-driven). Only on a genuinely fresh install.
  const [showWizard, setShowWizard] = useState<boolean>(
    () => features.firstRun && storageWritable() && !wizardSeen(),
  )
  const { commitLeft, commitRight, resetWidths } = usePaneWidths()
  const layoutRef = useRef<HTMLElement>(null)
  const [snap, setSnap] = useState<AppSnapshot | null>(null)
  const [view, setView] = useState<View>(() => {
    const h = typeof window !== 'undefined' ? window.location.hash.slice(1) : ''
    const sectionIds = sectionFeatures().map((f) => f.id) as string[]
    // Honor a deeplink only if it's an enabled section; otherwise open at the
    // active profile's landing view.
    if (sectionIds.includes(h) && features.enabled[h as FeatureId] !== false) {
      return h as View
    }
    return features.landing
  })
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

  // Top-level operating AREA: 'dx' (FT8/FT4 structured), 'msg' (FT1/DX1 chat), or
  // 'connect' (the situational-awareness map + propagation surface). The pill tabs
  // swap the nav. DX/MSG also bind the radio tier+mode; 'connect' is awareness-only
  // and must NOT retune the radio. Default DX (the 80% case).
  const [area, setArea] = useState<'dx' | 'msg' | 'connect'>(() => {
    try {
      const v = localStorage.getItem('nexus.workspace')
      if (v === 'dx' || v === 'msg' || v === 'connect') return v
    } catch {
      /* unreadable — fall through */
    }
    return 'dx'
  })
  // Sync the engine to the persisted area once on load (atomic tier+mode). Connect
  // doesn't bind a tier, so never push it to the engine.
  const areaSyncedRef = useRef(false)
  useEffect(() => {
    if (areaSyncedRef.current || !snap) return
    areaSyncedRef.current = true
    if (area !== 'connect') void apiSetArea(area).then((s) => s && setSnap(s))
  }, [snap, area])

  const handleWorkspace = useCallback((w: 'dx' | 'msg' | 'connect') => {
    setArea(w)
    try {
      localStorage.setItem('nexus.workspace', w)
    } catch {
      /* ignore */
    }
    setView(w === 'connect' ? 'connect' : w === 'dx' ? 'operate' : 'chat')
    // Connect is awareness-only: leave the radio on whatever tier it was.
    if (w === 'connect') return
    void withErrorToast(() => apiSetArea(w), 'Could not switch area').then((s) => {
      if (s) setSnap(s)
    })
  }, [])
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
  // Need-tier per heard call (top tag), so the roster can colour wanted stations.
  // Same scoring the Propagation "need heard now" list uses (get_need_alerts), keyed
  // by callsign for the roster; refreshed on the prop cadence.
  const [needByCall, setNeedByCall] = useState<Map<string, NeedTag>>(new Map())
  useEffect(() => {
    let live = true
    const load = () =>
      getNeedAlerts()
        .then((alerts) => {
          if (!live) return
          const m = new Map<string, NeedTag>()
          for (const a of alerts) {
            if (a.tags.length > 0) m.set(a.call.toUpperCase(), a.tags[0])
          }
          setNeedByCall(m)
        })
        .catch(() => {})
    load()
    const id = setInterval(load, 30_000)
    return () => {
      live = false
      clearInterval(id)
    }
  }, [])
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
    void withErrorToast(() => Promise.resolve(apiSelectPeer(call)), 'Could not select station')
  }, [])

  // The Map and the roster share ONE selection: the active peer. Clicking a map
  // dot selects (or, if already selected, clears) that station — and the roster
  // highlights it too, since StationList already keys its highlight off activePeer.
  const handleMapSelect = useCallback((call: string | null) => {
    void withErrorToast(() => Promise.resolve(apiSelectPeer(call)), 'Could not select station')
  }, [])

  const handleCall = useCallback(
    (call: string, grid?: string, message?: string, snr?: number) => {
      void withErrorToast(
        () => apiCallStation(call, grid, message, snr),
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

  const handleConfirmLog = useCallback((record: LoggedQso) => {
    void withErrorToast(() => apiConfirmPendingLog(record), 'Could not log QSO').then((s) => {
      if (s) setSnap(s)
    })
  }, [])

  const handleDiscardLog = useCallback(() => {
    void withErrorToast(() => apiDiscardPendingLog(), 'Could not discard QSO').then((s) => {
      if (s) setSnap(s)
    })
  }, [])

  const handleSend = useCallback(
    (text: string) => {
      if (!activePeer) return
      void withErrorToast(
        () => Promise.resolve(apiSendMessage(activePeer, text)),
        'Message could not be sent',
      )
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
  const handleTune = useCallback((hz: number, shift: boolean) => {
    const call = shift ? () => apiSetTxOffset(hz) : () => apiSetRxOffset(hz)
    void withErrorToast(call, 'Could not set offset').then((s) => {
      if (s) setSnap(s)
    })
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
      }
    })
  }, [])

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
      else if (next === 'qso') handleSetMode('qso-monitor')
      else if (next === 'fieldDay') handleSetMode('fieldday-sp')
    },
    [handleSetMode],
  )

  const handleTier = useCallback((t: Tier) => {
    void withErrorToast(() => apiSetTier(t), 'Could not change tier').then((s) => {
      if (s) setSnap(s)
    })
  }, [])

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
    (ids: ProfileId[], landing: View) => {
      features.applyProfiles(ids)
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
        <span>Connecting to Tempo…</span>
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
    (snap.mycall.trim() === '' || snap.mycall.trim().toUpperCase() === PLACEHOLDER_CALL)

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

  let workspace: JSX.Element
  switch (effectiveView) {
    case 'qso':
      workspace = threePane(
        <QsoPanel
          qso={snap.qso}
          onSetMode={handleSetMode}
          onResend={handleQsoResend}
          onFreetext={handleQsoFreetext}
          onWork={handleCall}
        />,
      )
      break
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
    case 'awards':
      workspace = (
        <main className="layout single">
          <AwardsView showGamification={features.isOn('gamification')} />
        </main>
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
      workspace = (
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
          roster={stationsPanel}
          needByCall={needByCall}
          selectedCall={activePeer}
          onSelect={handleSelect}
          layoutMode={operateLayout}
          onLayoutMode={handleOperateLayout}
        />
      )
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
        />
      )
      break
    case 'propagation':
      workspace = (
        <main className="layout single">
          <PropagationView snap={prop} />
        </main>
      )
      break
    case 'map':
      workspace = (
        <main className="layout single">
          <MapView
            myGrid={settings?.mygrid ?? ''}
            theme={theme}
            stations={snap?.stations ?? []}
            prop={prop}
            selectedCall={activePeer}
            onSelectCall={handleMapSelect}
            needByCall={needByCall}
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
        propEnabled={features.isOn('propagation')}
        onNavigate={handleView}
      />

      <div className="shell">
        <ModeNav
          view={effectiveView}
          mode={snap.mode}
          enabled={features.enabled}
          onSelect={handleView}
          workspace={area}
          onWorkspace={handleWorkspace}
        />
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
