import { useEffect, useRef, useState } from 'react'
import type { AppSnapshot, FieldDayStatus, NeedTag, Settings, SpotRow } from '../types'
import { PhoneScope } from './PhoneScope'
import { PalettePicker } from './PalettePicker'
import { BandPicker } from './BandPicker'
import { BandStrip } from './BandStrip'
import { TuningStrip } from './TuningStrip'
import { CockpitHeader } from './CockpitHeader'
import { Splitter } from './Splitter'
import { LogEntry } from './LogEntry'
import {
  getSettings,
  setSettings,
  sendCw,
  setCwKeyer,
  setCwWpm,
  stopCw,
  cwDecode,
  cwClear,
  setAiCw,
  selectPeer,
  previewCw,
  pointRotatorAtCall,
  setRigFunc,
  setFilterWidth,
  setNrLevel,
  setAgc,
  setScopeSpan,
  setScopeRef,
  setFlexPanSpan,
  setFlexPanRef,
  openPanelWindow,
  setTune,
  setFrequency,
  haltTx,
} from '../api'
import { bandLabelForMhz } from '../band'
import { pushToast, withErrorToast } from '../toast'
import { RotorStrip } from './RotorStrip'
import { useWheelTune } from '../useWheelTune'
import { useScopeTune } from '../useScopeTune'
import { isRfScopeSource, sidebandSign } from '../waterfall'

/** Client-side RF-zoom presets for a native panadapter (mirror of the Phone cockpit). */
const RF_SPANS = [
  { label: 'Full', lo: -1e9, hi: 1e9, title: "The rig's whole scope sweep (set the width on the radio)" },
  { label: '±25k', lo: -25_000, hi: 25_000, title: '±25 kHz around your dial' },
  { label: '±10k', lo: -10_000, hi: 10_000, title: '±10 kHz around your dial' },
  { label: '±5k', lo: -5_000, hi: 5_000, title: '±5 kHz around your dial' },
] as const

/** RIG scope-span presets (native Icom CI-V) — command the RADIO's real panadapter sweep width
 *  via CI-V 27 15, exactly as the Phone cockpit does. */
const RIG_SPANS = [
  { label: '±2.5k', hz: 2_500 },
  { label: '±5k', hz: 5_000 },
  { label: '±10k', hz: 10_000 },
  { label: '±25k', hz: 25_000 },
  { label: '±50k', hz: 50_000 },
  { label: '±100k', hz: 100_000 },
  { label: '±250k', hz: 250_000 },
] as const

/** FlexRadio pan BANDWIDTH presets (full span) — command the SmartSDR panadapter width. */
const FLEX_SPANS = [
  { label: '50k', hz: 50_000 },
  { label: '100k', hz: 100_000 },
  { label: '200k', hz: 200_000 },
  { label: '500k', hz: 500_000 },
  { label: '1M', hz: 1_000_000 },
  { label: '2M', hz: 2_000_000 },
] as const

interface Props {
  snap: AppSnapshot
  theme: string
  /** CW sidetone pitch (Hz) — the scope's zero-beat marker. */
  pitchHz?: number
  /** Wheel-tune sensitivity (from Settings) — applied to the scope + readout wheel-tune. */
  wheelSensitivity?: number
  /** Click-to-work handoff from the Needed board: the callsign to prefill the log with.
   * `ts` changes on each click so re-working the same call refires the prefill. */
  pendingWork?: { call: string; ts: number } | null
  /** Called once the prefill has been applied, so the parent can clear it. */
  onConsumeWork?: () => void
  /** Apply a snapshot returned by a command (e.g. a band QSY) without waiting for the poll. */
  onSnap?: (snap: AppSnapshot) => void
  /** Field Day status — when non-null the log strip switches to FD mode. */
  fieldDay?: FieldDayStatus | null
  /** Live cluster spots (all bands/modes); the band-strip filters to CW on the current band. */
  spots?: SpotRow[]
  /** Top need tag per heard call (UPPERCASE) — colours band-strip ticks by need tier. */
  needByCall?: Map<string, NeedTag>
  /** Activity type per heard call (UPPERCASE) — POTA/SOTA/DXped badges on the band strip. */
  typeByCall?: Map<string, 'Pota' | 'Sota' | 'Dxped'>
  /** Work a spotted station from the band-strip (QSY to its freq + prefill the log). */
  onWorkSpot?: (s: SpotRow) => void
}

/** Default CASUAL/ragchew macro set (no contest serial/exchange). Standard CW QSO flow:
 * F1 CQ → F2 Call (answer a CQ with just your call, so they copy it — no report yet) →
 * F3 Reply (send your report + name, once they've come back to you) → F4 73. Overs end
 * `KN` ("go ahead, you only"). The engine expands the tokens ({MYCALL}/{NAME}/{RST}/! =
 * worked call) with the live QSO context, so we just send the template. */
const DEFAULT_MACROS: { key: string; label: string; text: string }[] = [
  { key: 'F1', label: 'CQ', text: 'CQ CQ DE {MYCALL} {MYCALL} K' },
  { key: 'F2', label: 'Call', text: '! DE {MYCALL} {MYCALL} K' },
  { key: 'F3', label: 'Reply', text: '! DE {MYCALL} UR {RST} {RST} NAME {NAME} {NAME} HW? KN' },
  { key: 'F4', label: '73', text: '! DE {MYCALL} TU 73 SK' },
  { key: 'F5', label: 'My Call', text: '{MYCALL}' },
  { key: 'F6', label: 'His Call', text: '! ' },
  { key: 'F7', label: 'AGN', text: 'AGN AGN' },
  { key: 'F8', label: '?', text: '? ' },
]

/** Default Field Day CW macro set — replaces the casual defaults while FD mode is on.
 * The engine fills {EXCH} = "{CLASS} {SECTION}" (e.g. "3A WI") from the FD settings, so
 * one template serves both events. Contest cadence: F1 CQ FD → F2 answer with your call →
 * F3 send the exchange (twice, for copy) → F4 confirm + TU. */
const DEFAULT_FD_MACROS: { key: string; label: string; text: string }[] = [
  { key: 'F1', label: 'CQ FD', text: 'CQ FD DE {MYCALL} {MYCALL} K' },
  { key: 'F2', label: 'Call', text: '! DE {MYCALL} K' },
  { key: 'F3', label: 'Exch', text: '! DE {MYCALL} {EXCH} {EXCH} K' },
  { key: 'F4', label: 'TU', text: '! TU {EXCH} DE {MYCALL} K' },
  { key: 'F5', label: 'My Call', text: '{MYCALL}' },
  { key: 'F6', label: 'His Call', text: '! ' },
  { key: 'F7', label: 'AGN', text: 'AGN AGN' },
  { key: 'F8', label: '?', text: '? ' },
]

const WPM_MIN = 5
const WPM_MAX = 50

/**
 * CW operating cockpit — casual/ragchew. Keyboard + F-key macros key the rig via the
 * CAT keyer (the engine's send_cw path); the waterfall is the CW spectrum; a compact
 * strip logs the QSO into the multi-mode logbook (RST 599). Entering the section forces
 * the rig to CW (the rig-mode policy, wired in App). No contest scoring — by design.
 */
/** DSP funcs relevant to CW — NB (impulse noise), NR (broadband hiss), Notch/ANF (carriers).
 * COMP/VOX are voice-only, so they're deliberately absent here. Capability-gated like Phone. */
const CW_DSP_FUNCS = [
  { key: 'nb', label: 'NB', title: 'Noise Blanker — kills impulse/ignition noise (RX)' },
  { key: 'nr', label: 'NR', title: 'Noise Reduction — pulls a tone out of broadband hiss (RX, DSP)' },
  { key: 'notch', label: 'Notch', title: 'Auto-Notch (ANF) — nulls a competing carrier (RX, DSP)' },
] as const

export function CwCockpit({
  snap,
  theme,
  pitchHz = 600,
  wheelSensitivity,
  pendingWork,
  onConsumeWork,
  onSnap,
  fieldDay,
  spots,
  needByCall,
  typeByCall,
  onWorkSpot,
}: Props) {
  const catOk = snap.radio.catOk === true
  // Wheel-to-tune over the CW scope, sharing the tuning strip's step selector.
  const [tuneStep, setTuneStep] = useState(100)
  const scopeRef = useRef<HTMLElement>(null)
  useWheelTune(scopeRef, {
    dialMhz: snap.radio.dialMhz,
    sideband: snap.radio.sideband || 'USB',
    enabled: catOk && !snap.radio.transmitting,
    stepHz: tuneStep,
    sensitivity: wheelSensitivity,
    onSnap,
  })
  // Click/drag tuning from the scope (Flex-style): a click zero-beats the clicked CW
  // signal to the pitch; a press-drag slides the passband box, coalesced ~120 ms.
  // The CAT write keeps the raw sideband (what wheel-tune sends — the CW rig-mode
  // policy is applied separately by the engine).
  const onScopeTune = useScopeTune({
    sideband: snap.radio.sideband || 'USB',
    enabled: catOk && !snap.radio.transmitting,
    onSnap,
  })
  // The scope's click/box math needs a CW-CLASSIFIED mode string (settings.sideband is
  // USB/LSB here — the soundcard keyer keys through SSB): same sideband SIGN, but the
  // click zero-beats instead of carrier-snapping, and the box centers on the dial.
  const scopeMode = sidebandSign(snap.radio.sideband || 'USB') < 0 ? 'CW-L' : 'CW'
  // RX filter width (CW wants a NARROW filter — default 500 Hz, 50-Hz steps, 50–2000 Hz span).
  const filterHz = snap.radio.filterWidthHz ?? null
  const bumpFilter = (deltaHz: number) => {
    const base = filterHz ?? 500
    const next = Math.min(2000, Math.max(50, base + deltaHz))
    // Never let the clamp invert the direction — "wider" must not narrow (e.g. a stale Phone
    // width above CW's 2 kHz cap right after switching modes, before the next `m` re-read).
    if ((deltaHz > 0 && next <= base) || (deltaHz < 0 && next >= base)) return
    void setFilterWidth(next)
      .then((s) => onSnap?.(s))
      .catch(() => pushToast('Could not set filter width', 'error'))
  }
  // Source of truth = the engine's actual keyer speed (survives navigation; the
  // old hard-coded 25 silently re-keyed at the wrong speed after a nav round-trip).
  const [wpm, setWpm] = useState(() => snap.radio.cwWpm ?? 25)
  useEffect(() => {
    if (snap.radio.cwWpm != null) setWpm(snap.radio.cwWpm)
  }, [snap.radio.cwWpm])
  // --- CI-V RX DSP + panadapter controls, at PARITY with the Phone cockpit (a CW op wants
  //     AGC speed, NR depth, and the rig's real panadapter just as much). Each control is
  //     capability-gated on what the rig actually reports, so nothing shows on a rig that lacks it.
  const [nr, setNr] = useState(30) // % noise-reduction depth — pushed once touched
  const nrDragging = useRef(false)
  useEffect(() => {
    const rb = snap.radio.nrLevel
    if (rb != null && !nrDragging.current) {
      const pct = Math.round(rb * 100)
      setNr((n) => (Math.abs(n - pct) >= 2 ? pct : n))
    }
  }, [snap.radio.nrLevel])
  const changeNr = (pct: number) => {
    setNr(pct)
    void setNrLevel(pct / 100)
  }
  // AGC speed — local optimistic mirror so the segmented highlight flips on click (same fix as
  // the Phone cockpit: snap.radio.agc lags a poll behind the click).
  const [agc, setAgcLocal] = useState<string | null>(() => snap.radio.agc ?? null)
  useEffect(() => {
    if (snap.radio.agc != null) setAgcLocal(snap.radio.agc)
  }, [snap.radio.agc])
  const changeAgc = (sp: 'fast' | 'mid' | 'slow') => {
    setAgcLocal(sp)
    void setAgc(sp)
      .then((s) => onSnap?.(s))
      .catch(() => {})
  }
  // Native scope feed (reported by PhoneScope) → drives the RF-panadapter switch, exactly like
  // Phone. When a Flex/Icom CI-V scope is streaming we show the real RF spectrum (with client
  // RF-zoom + the rig scope controls); otherwise the CW-narrow audio zero-beat view stays.
  const [scopeFeed, setScopeFeed] = useState<{ source: string; loHz: number; hiHz: number } | null>(
    null,
  )
  const nativeRf = scopeFeed != null && isRfScopeSource(scopeFeed.source)
  const civScope = scopeFeed?.source === 'civ'
  const flexScope = scopeFeed?.source === 'flex'
  const [flexRefDbm, setFlexRefDbm] = useState(-80)
  const changeFlexRef = (dbm: number) => {
    setFlexRefDbm(dbm)
    void setFlexPanRef(dbm)
      .then((s) => onSnap?.(s))
      .catch(() => {})
  }
  const [rfSpan, setRfSpan] = useState<(typeof RF_SPANS)[number]>(RF_SPANS[0])
  const [scopeRefTenths, setScopeRefTenths] = useState(0)
  const changeScopeRef = (tenths: number) => {
    setScopeRefTenths(tenths)
    void setScopeRef(tenths)
  }
  // Live single-signal CW decode of the receive audio at the marker pitch — poll the
  // engine ~1.4 Hz (the decode reads a multi-second ring, so faster adds no detail).
  const [decoded, setDecoded] = useState<{ text: string; wpm: number }>({ text: '', wpm: 0 })
  const decodeRef = useRef<HTMLDivElement>(null)
  // Cockpit root: the scope-height splitter measures + writes its CSS var here.
  const cockpitRef = useRef<HTMLElement>(null)
  // Decode sensitivity for the internal pitch decoder (now WPM-estimation + AI-off
  // fallback only — the slider left with the classic pane; the stored value still applies).
  const sensitivityRef = useRef<number>(
    (() => {
      const v = parseFloat(localStorage.getItem('nexus.cw.sensitivity') ?? '')
      return Number.isFinite(v) ? Math.min(1, Math.max(0, v)) : 0.5
    })(),
  )
  // Typewriter reveal: the AI decoder emits each inference pass's text as one batch
  // (window-batch inference — several characters land at once). Reveal appended text
  // character-by-character so the copy FLOWS like a live operator's; the drain rate
  // scales with backlog so a big batch clears in ~2 s and never falls behind. A
  // non-append change (transcript reset/trim) snaps to the full text instantly.
  const [revealLen, setRevealLen] = useState(0)
  const prevTextRef = useRef('')
  useEffect(() => {
    const text = decoded.text
    if (!text.startsWith(prevTextRef.current)) setRevealLen(text.length)
    prevTextRef.current = text
  }, [decoded.text])
  useEffect(() => {
    const backlog = decoded.text.length - revealLen
    if (backlog <= 0) return
    const id = window.setInterval(() => {
      setRevealLen((n) => Math.min(decoded.text.length, n + Math.max(1, Math.ceil((decoded.text.length - n) / 40))))
    }, 50)
    return () => window.clearInterval(id)
  }, [decoded.text, revealLen])
  const revealedText = decoded.text.slice(0, revealLen)
  // Keep the newest decoded text in view as the transcript grows.
  useEffect(() => {
    const el = decodeRef.current
    if (el) el.scrollTop = el.scrollHeight
  }, [revealedText])
  // TX echo — what we've actually transmitted (macros expanded), polled alongside the decode.
  const [sent, setSent] = useState<string[]>([])
  const sentRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const el = sentRef.current
    if (el) el.scrollTop = el.scrollHeight
  }, [sent])
  // A CW-keyer failure surfaced by the radio loop (e.g. the rig rejected CAT send_morse).
  const [keyerError, setKeyerError] = useState<string | null>(null)
  // --- CW copilot: decoded-call chips + guided next-step (configurable Guided/Expert) ---
  const [assistMode, setAssistMode] = useState<'guided' | 'expert'>(
    () => (localStorage.getItem('nexus.cwAssist') as 'guided' | 'expert') || 'guided',
  )
  const setAssist = (m: 'guided' | 'expert') => {
    setAssistMode(m)
    localStorage.setItem('nexus.cwAssist', m)
  }
  const [cand, setCand] = useState<{ call: string; best: boolean }[]>([])
  const [guide, setGuide] = useState<{
    state: string
    headline: string
    prompt: string
    recommended: string | null
    workedCall: string | null
    rst: string | null
    name: string | null
  }>({
    state: 'listening',
    headline: '',
    prompt: '',
    recommended: null,
    workedCall: null,
    rst: null,
    name: null,
  })
  // True once the operator sets WPM by hand → stop auto-matching to the decoded speed.
  const wpmTouched = useRef(false)
  // F-key macros come from the ACTIVE named CW profile (Settings ▸ Macros). A rotating
  // operator can switch profiles right here in the cockpit bar; an empty profile falls
  // back to the built-in defaults — which swap to the Field Day set (with the {EXCH}
  // exchange tokens) while FD mode is on. Keep the full settings so the switcher can
  // persist the new active-profile index without dropping other fields.
  const [cwSettings, setCwSettings] = useState<Settings | null>(null)
  const [profiles, setProfiles] = useState<{ name: string; macros: { key: string; label: string; text: string }[] }[]>(
    [],
  )
  const [activeProfile, setActiveProfile] = useState(0)
  useEffect(() => {
    let alive = true
    void getSettings()
      .then((s) => {
        if (!alive) return
        setCwSettings(s)
        setProfiles(s.macros?.cwProfiles ?? [])
        setActiveProfile(s.macros?.activeCwProfile ?? 0)
      })
      .catch(() => {})
    return () => {
      alive = false
    }
  }, [])
  const profileMacros = profiles[activeProfile]?.macros
  const macros =
    profileMacros && profileMacros.length ? profileMacros : fieldDay ? DEFAULT_FD_MACROS : DEFAULT_MACROS
  // Switch the active macro profile from the cockpit (optimistic) and persist it.
  const switchProfile = (i: number) => {
    setActiveProfile(i)
    if (!cwSettings) return
    const next = { ...cwSettings, macros: { ...cwSettings.macros, activeCwProfile: i } }
    setCwSettings(next)
    void setSettings(next)
      .then((s) => onSnap?.(s))
      .catch(() => pushToast('Could not switch macro profile', 'error'))
  }
  const macrosRef = useRef(macros)
  macrosRef.current = macros
  // Reply preview: the exact text each F-key WILL send (macros expanded with the worked
  // call). Refetched from the backend when the worked station changes — cheap, once per QSO.
  const [previews, setPreviews] = useState<Record<string, string>>({})
  useEffect(() => {
    let alive = true
    Promise.all(
      macros.map((m) =>
        previewCw(m.text)
          .then((p) => [m.key, p] as const)
          .catch(() => [m.key, m.text] as const),
      ),
    ).then((entries) => {
      if (alive) setPreviews(Object.fromEntries(entries))
    })
    return () => {
      alive = false
    }
  }, [guide.workedCall, macros])
  useEffect(() => {
    let alive = true
    const tick = () => {
      cwDecode(sensitivityRef.current)
        .then((d) => {
          if (alive) {
            setDecoded({ text: d.text, wpm: d.wpm })
            setSent(d.sent)
            setKeyerError(d.keyerError)
            setCand(d.candidates)
            setGuide({
              state: d.state,
              headline: d.headline,
              prompt: d.prompt,
              recommended: d.recommended,
              workedCall: d.workedCall,
              rst: d.rst,
              name: d.name,
            })
          }
        })
        .catch(() => {})
    }
    tick()
    // Poll the decoded transcript often — the Rust streaming decoder updates every ~20 ms, so a
    // slow poll is pure display lag (the "desktop lags the web decoder" report). 200 ms ≈ 5 Hz
    // keeps copy near real-time without hammering the command channel.
    const id = window.setInterval(tick, 200)
    return () => {
      alive = false
      window.clearInterval(id)
    }
  }, [])
  // Initialize the keyer toggle from the engine's ACTUAL setting (the snapshot is the source
  // of truth) — not a hard-coded 'cat'. A stale local default showed CAT while the backend was
  // on Soundcard, so CW silently went to USB (Soundcard keying = rig in SSB) with no clue why.
  const [keyer, setKeyer] = useState<'cat' | 'soundcard' | 'winkeyer' | 'serial'>(
    () => (snap.radio.cwKeyer as 'cat' | 'soundcard' | 'winkeyer' | 'serial') || 'cat',
  )
  // Keep it in sync if the backend value changes (or arrives after first render).
  useEffect(() => {
    if (snap.radio.cwKeyer) setKeyer(snap.radio.cwKeyer as 'cat' | 'soundcard' | 'winkeyer' | 'serial')
  }, [snap.radio.cwKeyer])
  const [text, setText] = useState('')
  // Sidetone pitch — local for instant marker response; persisted via set_cw_keyer.
  const [pitch, setPitch] = useState(pitchHz)
  useEffect(() => setPitch(pitchHz), [pitchHz])
  const changePitch = (v: number) => {
    const p = Math.max(300, Math.min(1200, Math.round(v)))
    setPitch(p)
    void setCwKeyer(keyer, p).then((s) => s && onSnap?.(s))
  }

  const changeWpm = (w: number) => {
    const v = Math.max(WPM_MIN, Math.min(WPM_MAX, Math.round(w)))
    setWpm(v)
    void setCwWpm(v)
  }
  // Live snapshot ref so the keyboard handler (bound once, `[]` deps) reads the CURRENT
  // TX-allowed privilege state through send() — not whatever existed at mount.
  const snapRef = useRef(snap)
  snapRef.current = snap
  const send = (t: string) => {
    if (!t.trim()) return
    // The engine blocks keying outside privileges anyway; surface why up front.
    if (!snapRef.current.radio.txAllowed) {
      pushToast('TX locked — this frequency is outside your license privileges', 'info', 3500)
      return
    }
    void withErrorToast(() => sendCw(t), 'CW send failed')
  }
  const sendTyped = () => {
    send(text)
    setText('')
  }
  const abort = () => {
    // Stop the CW keyer AND drop any tune carrier / stray PTT — a true stop-everything (Esc).
    void stopCw()
    void haltTx()
  }
  // Commit a typed dial from the shared header readout — same CAT path as the
  // TuningStrip nudge/wheel (keeps the current sideband so an in-band entry
  // never flips the mode); rejects out-of-plan frequencies with a toast.
  const commitDial = (mhz: number) => {
    const band = bandLabelForMhz(mhz)
    if (!band) {
      pushToast(`${mhz.toFixed(4)} MHz is outside the band plan`, 'error', 3000)
      return
    }
    void setFrequency(mhz, band, snap.radio.sideband || 'USB')
      .then((s) => s && onSnap?.(s))
      .catch(() => {})
  }
  const changeKeyer = (k: 'cat' | 'soundcard' | 'winkeyer' | 'serial') => {
    setKeyer(k)
    void setCwKeyer(k).then((s) => s && onSnap?.(s))
  }
  // Confirm a decoded station as the one we're working: make it the macro/log peer (so the
  // `!` token + logging use it), match our speed to theirs (unless WPM was set by hand), and
  // prefill the log with the read call/RST/name. Never transmits — the operator still keys.
  const workCall = (call: string) => {
    void selectPeer(call)
      .then((s) => s && onSnap?.(s))
      .catch(() => {})
    if (!wpmTouched.current && decoded.wpm >= WPM_MIN) changeWpm(decoded.wpm)
    // The log fills continuously from the confirmed worked station (see cwLive on LogEntry) —
    // call now, then RST + name as they're decoded through the QSO.
  }

  // A click-to-work / spot-click handoff (pendingWork) must ARM the macro peer, or the `!` macros
  // would key the previously-selected chip's call on the new station's frequency. Plain selectPeer
  // (NOT workCall) — don't speed-match to the old decoded WPM (a different signal). The log prefill
  // is handled separately by LogEntry.
  useEffect(() => {
    if (pendingWork?.call) {
      void selectPeer(pendingWork.call)
        .then((s) => s && onSnap?.(s))
        .catch(() => {})
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pendingWork?.ts])

  // Keyboard: F1–F8 fire macros; Esc aborts; PgUp/PgDn nudge speed (±2, Shift ±4).
  // Live ref so the document listener (bound once) always reads current state.
  const stateRef = useRef({ wpm, text })
  stateRef.current = { wpm, text }
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const macro = macrosRef.current.find((m) => m.key === e.key)
      if (macro) {
        e.preventDefault()
        send(macro.text)
      } else if (e.key === 'Escape') {
        e.preventDefault()
        abort()
      } else if (e.key === 'PageUp') {
        e.preventDefault()
        wpmTouched.current = true
        changeWpm(stateRef.current.wpm + (e.shiftKey ? 4 : 2))
      } else if (e.key === 'PageDown') {
        e.preventDefault()
        wpmTouched.current = true
        changeWpm(stateRef.current.wpm - (e.shiftKey ? 4 : 2))
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  return (
    <main className="layout single cw-cockpit" ref={cockpitRef}>
      <CockpitHeader
        snap={snap}
        onSnap={onSnap}
        modeIndicator={
          <span
            className="cw-mode-badge"
            title={snap.radio.catDetail || "The rig is set to CW while you're in this section"}
          >
            CW
          </span>
        }
        bandControl={<BandPicker snap={snap} mode="cw" onSnap={onSnap} />}
        onCommitDial={commitDial}
        wheelTune
        wheelStepHz={tuneStep}
        wheelSensitivity={wheelSensitivity}
        frequencyExtras={
          <TuningStrip
            snap={snap}
            onSnap={onSnap}
            step={tuneStep}
            onStep={setTuneStep}
            sensitivity={wheelSensitivity}
            showReadout={false}
          />
        }
        onTune={(on) => void setTune(on).then((s) => onSnap?.(s))}
        onStopTx={abort}
      >
        <label className="cw-wpm" title="Keyer speed — PgUp/PgDn to nudge (Shift = ±4)">
          <span>Speed</span>
          <input
            type="range"
            min={WPM_MIN}
            max={WPM_MAX}
            value={wpm}
            onChange={(e) => {
              wpmTouched.current = true
              changeWpm(Number(e.target.value))
            }}
            aria-label="CW keyer speed (WPM)"
          />
          <span className="cw-wpm-val">{wpm} WPM</span>
        </label>
        <div className="cw-keyer" role="group" aria-label="CW keyer back-end">
          <button
            type="button"
            className={`cw-keyer-opt${keyer === 'cat' ? ' active' : ''}`}
            onClick={() => changeKeyer('cat')}
            title="CAT keyer — the rig generates CW (rig in CW). Zero extra hardware."
          >
            CAT
          </button>
          <button
            type="button"
            className={`cw-keyer-opt${keyer === 'serial' ? ' active' : ''}`}
            onClick={() => changeKeyer('serial')}
            title="Serial keyline — Nexus toggles DTR/RTS into the rig's KEY jack (rig in CW, rig shapes the signal). The clean N1MM/fldigi method for rigs without CAT CW. Set the keyline port + line in Settings ▸ CW."
          >
            Serial
          </button>
          <button
            type="button"
            className={`cw-keyer-opt${keyer === 'winkeyer' ? ' active' : ''}`}
            onClick={() => changeKeyer('winkeyer')}
            title="K1EL WinKeyer — hardware keyer over serial (rig in CW). Set its port in Settings."
          >
            WinKeyer
          </button>
          <button
            type="button"
            className={`cw-keyer-opt${keyer === 'soundcard' ? ' active' : ''}`}
            onClick={() => changeKeyer('soundcard')}
            title="Soundcard keyer — a keyed audio tone through SSB (rig in USB). A workaround: works ONLY if Nexus's audio output is routed to the rig (like FT8) AND PTT works, and you must keep drive below ALC. WinKeyer or the serial keyline are the clean options."
          >
            Soundcard
          </button>
        </div>
        <label className="cw-wpm" title="Sidetone / zero-beat pitch (Hz) — the scope's dashed marker">
          <span>Pitch</span>
          <input
            type="number"
            className="settings-input cw-pitch"
            min={300}
            max={1200}
            step={10}
            value={pitch}
            onChange={(e) => changePitch(Number(e.target.value))}
            aria-label="CW pitch (Hz)"
          />
        </label>
        {profiles.length > 1 && (
          <label className="cw-wpm" title="CW macro profile — your active F-key set (edit sets in Settings ▸ Macros)">
            <span>Macros</span>
            <select
              className="settings-input"
              value={activeProfile}
              onChange={(e) => switchProfile(Number(e.target.value))}
              aria-label="CW macro profile"
            >
              {profiles.map((p, i) => (
                <option key={i} value={i}>
                  {p.name || `Profile ${i + 1}`}
                </option>
              ))}
            </select>
          </label>
        )}
        {catOk && (
          <div className="ph-filter" title="RX filter / passband width (CAT) — narrow to dig CW out of QRM">
            <span className="ph-filter-lbl">BW</span>
            <button
              type="button"
              className="ph-filter-step"
              onClick={() => bumpFilter(-50)}
              title="Narrower (−50 Hz)"
            >
              −
            </button>
            <span className="ph-filter-val mono">{filterHz ? `${filterHz}` : '—'}</span>
            <button
              type="button"
              className="ph-filter-step"
              onClick={() => bumpFilter(50)}
              title="Wider (+50 Hz)"
            >
              +
            </button>
          </div>
        )}
        <RotorStrip
          targetCall={guide.workedCall}
          onPointAt={(call) =>
            pointRotatorAtCall(call)
              .then((bearing) => pushToast(`Rotator → ${call}: ${Math.round(bearing)}°`, 'info'))
              .catch((e) => pushToast(`Rotator: ${e instanceof Error ? e.message : e}`, 'error'))
          }
        />
        {snap.radio.splitTxMhz != null && (
          <span className="cw-mode-badge" title={`Split — TX ${snap.radio.splitTxMhz.toFixed(4)} MHz`}>
            SPLIT ▲
          </span>
        )}
      </CockpitHeader>

      {keyerError && (
        <div className="cw-keyer-warn" role="alert">
          ⚠ {keyerError}
        </div>
      )}

      <section className="ph-scope-panel" ref={scopeRef} title="Scroll here to tune the VFO">
        <div className="ph-scope-head">
          {/* When a native panadapter drives the scope, name it honestly (real RF spectrum);
              otherwise it's the CW-narrow audio view for zero-beating. */}
          <span
            className="ph-scope-title"
            title={
              nativeRf
                ? 'Native RF panadapter — the real RF spectrum around your dial.'
                : 'Receiver AUDIO around your CW pitch (~300–1100 Hz) — tune a signal onto the dashed hairline to zero-beat it.'
            }
          >
            {nativeRf ? 'RF Panadapter' : 'CW audio'}{' '}
            <span className="ph-scope-sub">
              {nativeRf && scopeFeed
                ? `· ${(scopeFeed.loHz / 1e6).toFixed(4)}–${(scopeFeed.hiHz / 1e6).toFixed(4)} MHz`
                : '· zero-beat'}
            </span>
          </span>
          <span className="ph-scope-head-label">Colors</span>
          <PalettePicker />
        </div>
        {nativeRf && (
          // Native RF panadapter: client-side RF-width zoom around the dial (mirror of Phone).
          <div className="ph-span" role="group" aria-label="Panadapter zoom">
            {RF_SPANS.map((sp) => (
              <button
                key={sp.label}
                type="button"
                className={`theme-chip${rfSpan.label === sp.label ? ' active' : ''}`}
                aria-pressed={rfSpan.label === sp.label}
                title={sp.title}
                onClick={() => setRfSpan(sp)}
              >
                {sp.label}
              </button>
            ))}
          </div>
        )}
        {/* CW-narrow view (~300–1100 Hz) unless a native RF scope is streaming, in which case
            we show the real RF spectrum around the dial. The dashed hairline is YOUR pitch. */}
        <PhoneScope
          transmitting={snap.radio.transmitting}
          theme={theme}
          smeterDb={snap.radio.smeterDb}
          viewLoHz={nativeRf ? rfSpan.lo : 300}
          viewHiHz={nativeRf ? rfSpan.hi : 1100}
          markerHz={nativeRf ? undefined : pitch}
          sideband={scopeMode}
          dialHz={snap.radio.dialMhz > 0 ? Math.round(snap.radio.dialMhz * 1e6) : null}
          onFeed={(source, loHz, hiHz) => setScopeFeed({ source, loHz, hiHz })}
          onTune={onScopeTune}
          filterWidthHz={filterHz ?? 500}
          pitchHz={pitch}
          cwPitchRefDial={keyer !== 'soundcard'}
          interactive={catOk && !snap.radio.transmitting && snap.radio.dialMhz > 0}
        />
      </section>
      <Splitter
        axis="y"
        varName="--cw-scope-h"
        target={cockpitRef}
        storageKey="nexus.split.cw.scope"
        minPx={100}
        maxPx={420}
        defaultPct={22}
        label="scope height"
      />

      {/* Rig scope controls (native Icom CI-V only) — command the RADIO's real panadapter:
          span sets the hardware sweep width, ref sets weak-signal visibility. Parity with Phone. */}
      {civScope && (
        <div className="ph-rigscope" role="group" aria-label="Rig scope control">
          <span className="ph-rigscope-lbl" title="These command the radio's own scope, not just the on-screen zoom">
            Rig&nbsp;scope
          </span>
          <div className="ph-span">
            {RIG_SPANS.map((sp) => (
              <button
                key={sp.label}
                type="button"
                className="theme-chip"
                title={`Set the radio's scope span to ${sp.label}`}
                onClick={() => void setScopeSpan(sp.hz).then((s) => onSnap?.(s)).catch(() => {})}
              >
                {sp.label}
              </button>
            ))}
          </div>
          <label className="ph-rigscope-ref" title="Scope reference level — lower to lift weak signals out of the noise">
            <span>Ref</span>
            <input
              type="range"
              min={-200}
              max={200}
              step={5}
              value={scopeRefTenths}
              onChange={(e) => changeScopeRef(Number(e.target.value))}
              aria-label="Scope reference level (dB)"
            />
            <span className="ph-power-val">{(scopeRefTenths / 10).toFixed(1)} dB</span>
          </label>
        </div>
      )}

      {/* FlexRadio SmartSDR panadapter controls — bandwidth + reference. Parity with Phone. */}
      {flexScope && (
        <div className="ph-rigscope" role="group" aria-label="Flex panadapter control">
          <span className="ph-rigscope-lbl" title="These command the FlexRadio's real SmartSDR panadapter, not just the on-screen zoom">
            Flex&nbsp;pan
          </span>
          <div className="ph-span">
            {FLEX_SPANS.map((sp) => (
              <button
                key={sp.label}
                type="button"
                className="theme-chip"
                title={`Set the Flex panadapter bandwidth to ${sp.label}`}
                onClick={() => void setFlexPanSpan(sp.hz).then((s) => onSnap?.(s)).catch(() => {})}
              >
                {sp.label}
              </button>
            ))}
          </div>
          <label className="ph-rigscope-ref" title="Panadapter reference level (dBm) — lower to lift weak signals out of the noise">
            <span>Ref</span>
            <input
              type="range"
              min={-140}
              max={-20}
              step={5}
              value={flexRefDbm}
              onChange={(e) => changeFlexRef(Number(e.target.value))}
              aria-label="Flex panadapter reference level (dBm)"
            />
            <span className="ph-power-val">{flexRefDbm} dBm</span>
          </label>
        </div>
      )}

      {/* DSP toggles (NB/NR/Notch) — capability-gated; only funcs the rig reports render. */}
      {(() => {
        const supported = CW_DSP_FUNCS.filter((f) => snap.radio[f.key] != null)
        if (supported.length === 0) return null
        return (
          <div className="ph-dsp" role="group" aria-label="Rig DSP functions">
            <span className="ph-dsp-label">DSP</span>
            {supported.map((f) => {
              const on = snap.radio[f.key] === true
              return (
                <button
                  key={f.key}
                  type="button"
                  className={`ph-dsp-btn${on ? ' on' : ''}`}
                  aria-pressed={on}
                  title={f.title}
                  onClick={() =>
                    void setRigFunc(f.key, !on)
                      .then((s) => onSnap?.(s))
                      .catch(() => pushToast(`Could not toggle ${f.label}`, 'error'))
                  }
                >
                  {f.label}
                </button>
              )
            })}
          </div>
        )
      })()}

      {/* RX DSP levels — NR depth + AGC speed, each shown only when the rig reports it. Parity
          with the Phone cockpit; a CW op leans on AGC speed and NR depth heavily. */}
      {(snap.radio.nrLevel != null || snap.radio.agc != null) && (
        <div className="ph-dsp-levels" role="group" aria-label="RX DSP levels">
          {snap.radio.nrLevel != null && (
            <label className="ph-dsplev" title="Noise-reduction depth — raise until the noise floor drops, back off if the tone gets watery">
              <span>NR</span>
              <input
                type="range"
                min={0}
                max={100}
                value={nr}
                onChange={(e) => changeNr(Number(e.target.value))}
                onPointerDown={() => {
                  nrDragging.current = true
                }}
                onPointerUp={() => {
                  nrDragging.current = false
                }}
                aria-label="Noise-reduction level"
              />
              <span className="ph-power-val">{nr}%</span>
            </label>
          )}
          {snap.radio.agc != null && (
            <div className="ph-agc" role="group" aria-label="AGC speed" title="AGC time constant — Fast for CW/pileups, Slow for steady copy">
              <span className="ph-dsplev-lbl">AGC</span>
              {(['fast', 'mid', 'slow'] as const).map((sp) => (
                <button
                  key={sp}
                  type="button"
                  className={`theme-chip${agc === sp ? ' active' : ''}`}
                  aria-pressed={agc === sp}
                  onClick={() => changeAgc(sp)}
                >
                  {sp === 'fast' ? 'Fast' : sp === 'mid' ? 'Mid' : 'Slow'}
                </button>
              ))}
            </div>
          )}
        </div>
      )}

      {/* CW spot band-activity strip; ⧉ pops the vertical band map into its own window. */}
      {onWorkSpot && (
        <BandStrip
          band={snap.radio.band}
          dialMhz={snap.radio.dialMhz}
          txAllowed={snap.radio.txAllowed}
          spots={spots ?? []}
          spotMode="CW"
          needByCall={needByCall}
          typeByCall={typeByCall}
          onWorkSpot={onWorkSpot}
          onPopOut={() => void openPanelWindow('bandmapCw')}
        />
      )}

      {/* CW copilot — decoded-call chips + (Guided) the next-step prompt. Configurable for
          new hams (Guided: plain-English prompts + the next key highlighted) vs experienced
          ops (Expert: just the chips). Nothing here transmits — the operator always keys. */}
      <div className={`cw-copilot ${assistMode}`}>
        <div className="cw-copilot-mode" role="group" aria-label="CW assist mode">
          <button
            type="button"
            className={assistMode === 'guided' ? 'active' : ''}
            onClick={() => setAssist('guided')}
            title="Guided: plain-English prompts + the next key highlighted — great if you don't know CW"
          >
            Guided
          </button>
          <button
            type="button"
            className={assistMode === 'expert' ? 'active' : ''}
            onClick={() => setAssist('expert')}
            title="Expert: just the decoded-call chips, no prompts"
          >
            Expert
          </button>
        </div>
        {assistMode === 'guided' && guide.headline && (
          <div className="cw-copilot-guide">
            <span className="cw-copilot-state">{guide.headline}</span>
            {guide.prompt && <span className="cw-copilot-prompt">{guide.prompt}</span>}
            {guide.recommended && previews[guide.recommended] && (
              <span className="cw-copilot-preview" title="What that key will transmit">
                → sends: {previews[guide.recommended]}
              </span>
            )}
          </div>
        )}
        <div className="cw-copilot-chips">
          {guide.workedCall ? (
            <span className="cw-copilot-label">Working</span>
          ) : cand.length > 0 ? (
            <span className="cw-copilot-label">Heard</span>
          ) : (
            <span className="cw-copilot-label dim">Decoded calls appear here…</span>
          )}
          {guide.workedCall && (
            <span className="cw-chip worked" title="The station you're working — the F-keys + log use this">
              {guide.workedCall}
              {guide.rst ? ` · ${guide.rst}` : ''}
              {guide.name ? ` · ${guide.name}` : ''}
            </span>
          )}
          {cand
            .filter((c) => c.call !== guide.workedCall)
            .map((c) => (
              <button
                key={c.call}
                type="button"
                className={`cw-chip${c.best ? ' best' : ''}`}
                onClick={() => workCall(c.call)}
                title={`Work ${c.call} — set it for the F-keys + log`}
              >
                {c.call}
              </button>
            ))}
        </div>
      </div>

      <div
        className="cw-decode"
        title="Live CW decode — the AI (neural-net) decoder reads the whole 400–1200 Hz window, far better weak-signal copy than a pitch-tracking decoder. Turn AI off to fall back to the classic decoder."
      >
        <div className="cw-decode-head">
          <span className="cw-decode-label">DECODE</span>
          <span className="cw-ai-beta">AI</span>
          {decoded.wpm > 0 && <span className="cw-decode-wpm">{decoded.wpm} WPM</span>}
          {/* Toggle is parked next to the label cluster on the LEFT and stays put — it must
              render BEFORE the (optional) AI status, or the status's auto-margin would shove
              the toggle to mid-row whenever the status text comes and goes. */}
          <button
            type="button"
            role="switch"
            aria-checked={snap.aiCw?.enabled ?? false}
            className={`toggle${snap.aiCw?.enabled ? ' on' : ''}`}
            onClick={() => void setAiCw(!(snap.aiCw?.enabled ?? false))}
            title={
              snap.aiCw?.enabled
                ? 'AI decoder on — click for the classic pitch decoder'
                : 'AI decoder off (classic pitch decoder) — click to turn AI on'
            }
          >
            <span className="toggle-knob" />
          </button>
          {snap.aiCw?.enabled && snap.aiCw.status && (
            <span className="cw-ai-status">{snap.aiCw.status}</span>
          )}
          <button
            className="cw-decode-clear"
            onClick={() => {
              void cwClear()
              setDecoded({ text: '', wpm: 0 })
              setSent([])
            }}
            title="Clear the decoded + sent transcript"
          >
            Clear
          </button>
        </div>
        {/* The visible transcript animates character-by-character (typewriter) —
            aria-hidden so a screen reader doesn't announce every keystroke; the
            hidden role=log mirror below receives whole batches instead (a log
            region announces only ADDITIONS). */}
        <div className="cw-decode-text" ref={decodeRef} aria-hidden="true">
          {revealedText ? (
            revealedText
          ) : (
            <span className="cw-decode-idle">
              {(snap.aiCw?.enabled && snap.aiCw.status) || 'listening…'}
            </span>
          )}
        </div>
        <div className="sr-only" role="log" aria-label="Decoded CW">
          {decoded.text}
        </div>
      </div>

      {sent.length > 0 && (
        <div
          className="cw-decode cw-sent-panel"
          title="What you've transmitted (F-key macros expanded to the real text)"
        >
          <div className="cw-decode-head">
            <span className="cw-decode-label">SENT ▲</span>
          </div>
          <div className="cw-decode-text" ref={sentRef}>
            {sent.map((line, i) => (
              <div key={i} className="cw-sent-line">
                {line}
              </div>
            ))}
          </div>
        </div>
      )}

      <div className="cw-macros" role="group" aria-label="CW macros">
        {macros.map((m) => (
          <button
            key={m.key}
            type="button"
            className={`cw-macro${assistMode === 'guided' && guide.recommended === m.key ? ' recommended' : ''}`}
            onClick={() => send(m.text)}
            title={previews[m.key] || m.text}
          >
            <span className="cw-macro-key">{m.key}</span>
            <span className="cw-macro-label">{m.label || m.key}</span>
          </button>
        ))}
      </div>

      <div className="cw-send">
        <input
          className="settings-input cw-type"
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault()
              sendTyped()
            }
          }}
          placeholder="Type CW to send… (Enter)"
          autoComplete="off"
          spellCheck={false}
        />
        <button type="button" className="cw-send-btn" onClick={sendTyped} disabled={!text.trim()}>
          Send
        </button>
      </div>

      <LogEntry
        snap={snap}
        mode="CW"
        defaultRst="599"
        pendingWork={pendingWork}
        onConsumeWork={onConsumeWork}
        cwLive={{
          call: guide.workedCall ?? cand.find((c) => c.best)?.call ?? null,
          rst: guide.rst,
          name: guide.name,
          confirmed: guide.workedCall != null,
        }}
        fieldDay={fieldDay}
        fdMode="CW"
      />
    </main>
  )
}
