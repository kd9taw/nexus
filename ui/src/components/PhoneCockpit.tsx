import { useEffect, useState, useRef } from 'react'
import type { AppSnapshot, FieldDayStatus, NeedTag, SpotRow } from '../types'
import { PhoneScope } from './PhoneScope'
import { TxMeters } from './TxMeters'
import { BandStrip } from './BandStrip'
import { SpotDialog } from './SpotDialog'
import { TuningStrip } from './TuningStrip'
import { Splitter } from './Splitter'
import { PalettePicker } from './PalettePicker'
import { BandPicker } from './BandPicker'
import { VoiceKeyer } from './VoiceKeyer'
import { LevelMeter } from './LevelMeter'
import { LogEntry } from './LogEntry'
import {
  setPtt,
  setRfPower,
  setMicGain,
  setNrLevel,
  setAgc,
  setScopeSpan,
  setScopeRef,
  setFlexPanSpan,
  setFlexPanRef,
  startQsoRecording,
  stopQsoRecording,
  setTune,
  haltTx,
} from '../api'
import { pushToast } from '../toast'
import { RotorStrip } from './RotorStrip'
import { MemoryBank } from './MemoryBank'
import { setFrequency, getSettings, setSettings, setSplit, setRigFunc, setSidebandOverride, setFilterWidth, openPanelWindow } from '../api'
import { bandLabelForMhz } from '../band'
import { isRfScopeSource } from '../waterfall'
import { useWheelTune } from '../useWheelTune'
import { useScopeTune } from '../useScopeTune'

interface Props {
  snap: AppSnapshot
  theme: string
  /** Click-to-work handoff from the Needed board: the callsign to prefill the log with.
   * `ts` changes on each click so re-working the same call refires the prefill. */
  pendingWork?: { call: string; ts: number } | null
  /** Called once the prefill has been applied, so the parent can clear it. */
  onConsumeWork?: () => void
  /** Apply a fresh snapshot returned by a command (so the REC toggle updates instantly
   * instead of waiting for the next poll). */
  onSnap?: (snap: AppSnapshot) => void
  /** Field Day status — when non-null the log strip switches to FD mode. */
  fieldDay?: FieldDayStatus | null
  /** Phone sub-mode from Settings ('ssb' | 'fm') — drives the mode badge, mirroring
   * the rig-mode policy's Phone arm (FM, else sideband by band). */
  phoneMode?: string
  /** Live cluster spots (all bands/modes); the band-strip filters to SSB on the current band. */
  spots?: SpotRow[]
  /** Top need tag per heard call (UPPERCASE) — colours band-strip ticks by need tier. */
  needByCall?: Map<string, NeedTag>
  /** Activity type per heard call (UPPERCASE) — POTA/SOTA/DXped badges on the band strip. */
  typeByCall?: Map<string, 'Pota' | 'Sota' | 'Dxped'>
  /** Work a spotted station from the band-strip (QSY to its freq + prefill the log). */
  onWorkSpot?: (s: SpotRow) => void
}

/**
 * Phone (voice) operating cockpit — casual/ragchew. The voice is the signal, so the
 * app does rig control + PTT + logging (you talk into the rig's mic; the live-mic
 * audio bridge + voice keyer land in P3-b/c). Entering forces USB/LSB by band (the
 * rig-mode keystone, wired in App).
 */
/** Expert DSP-function toggles. `key` matches the RadioStatus field + the set_rig_func name; the
 * cockpit only renders those the rig reports as supported (field non-null), so no dead buttons. */
const DSP_FUNCS = [
  { key: 'nb', label: 'NB', title: 'Noise Blanker — kills impulse/ignition noise (RX)' },
  { key: 'nr', label: 'NR', title: 'Noise Reduction — pulls voice out of broadband hiss (RX, DSP)' },
  { key: 'notch', label: 'Notch', title: 'Auto-Notch (ANF) — nulls carriers/heterodynes (RX, DSP)' },
  { key: 'comp', label: 'COMP', title: 'Speech Compressor — more average talk power (TX)' },
  { key: 'vox', label: 'VOX', title: 'Voice-Operated Transmit — hands-free keying (TX)' },
] as const

/** Bandscope span presets — slices of the captured audio passband. */
const SPANS = [
  { label: 'Full', lo: 0, hi: 4000, title: 'Whole captured band (0–4000 Hz) — incl. the filter slopes' },
  { label: 'Voice', lo: 300, hi: 2700, title: 'Voice energy (300–2700 Hz)' },
  { label: 'Low', lo: 200, hi: 1500, title: 'Lower half — zoomed (200–1500 Hz)' },
  { label: 'High', lo: 1500, hi: 2900, title: 'Upper half — zoomed (1500–2900 Hz)' },
] as const

/** RF panadapter zoom presets (used only when a native RF scope is streaming). Symmetric ±Hz
 *  windows centered on the dial — scopeView maps these to absolute RF and clamps to the swept
 *  span, so "Full" (a huge window) shows the rig's WHOLE sweep rather than a passband-width sliver. */
const RF_SPANS = [
  { label: 'Full', lo: -1e9, hi: 1e9, title: "The rig's whole scope sweep (set the width on the radio)" },
  { label: '±25k', lo: -25_000, hi: 25_000, title: '±25 kHz around your dial' },
  { label: '±10k', lo: -10_000, hi: 10_000, title: '±10 kHz around your dial' },
  { label: '±5k', lo: -5_000, hi: 5_000, title: '±5 kHz around your dial' },
] as const

/** RIG scope-span presets (native Icom CI-V only) — these change the RADIO's real panadapter
 *  sweep width via CI-V 27 15 (± half-width in Hz), from the rig's own span table. Unlike the
 *  client-side RF zoom above, this commands the hardware. */
const RIG_SPANS = [
  { label: '±2.5k', hz: 2_500 },
  { label: '±5k', hz: 5_000 },
  { label: '±10k', hz: 10_000 },
  { label: '±25k', hz: 25_000 },
  { label: '±50k', hz: 50_000 },
  { label: '±100k', hz: 100_000 },
  { label: '±250k', hz: 250_000 },
] as const

/** FlexRadio pan BANDWIDTH presets (full span, not ± half-width) — command the SmartSDR
 *  panadapter's real width via `display pan set … bw=`. */
const FLEX_SPANS = [
  { label: '50k', hz: 50_000 },
  { label: '100k', hz: 100_000 },
  { label: '200k', hz: 200_000 },
  { label: '500k', hz: 500_000 },
  { label: '1M', hz: 1_000_000 },
  { label: '2M', hz: 2_000_000 },
] as const

export function PhoneCockpit({ snap, theme, pendingWork, onConsumeWork, onSnap, fieldDay, phoneMode, spots, needByCall, typeByCall, onWorkSpot }: Props) {
  const [power, setPower] = useState(100) // % — only pushed to the rig once touched
  // Mirror the RIG's real level (CAT read-back / last commanded) so the slider
  // never lies at a guessed 100% — but never fight an in-flight drag.
  const dragging = useRef(false)
  useEffect(() => {
    const rb = snap.radio.rfPower
    if (rb != null && !dragging.current) {
      const pct = Math.round(rb * 100)
      setPower((p) => (Math.abs(p - pct) >= 2 ? pct : p))
    }
  }, [snap.radio.rfPower])
  const [mic, setMic] = useState(50) // % mic gain — pushed to the rig once touched
  const micDragging = useRef(false)
  useEffect(() => {
    const rb = snap.radio.micGain
    if (rb != null && !micDragging.current) {
      const pct = Math.round(rb * 100)
      setMic((m) => (Math.abs(m - pct) >= 2 ? pct : m))
    }
  }, [snap.radio.micGain])
  const changeMic = (pct: number) => {
    setMic(pct)
    void setMicGain(pct / 100)
  }
  const [nr, setNr] = useState(30) // % noise-reduction level — pushed once touched
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
  // AGC speed — local optimistic mirror so the segmented highlight flips on click. Reading
  // snap.radio.agc directly lagged ~0.75–1.5 s: setAgc's snapshot returns the OLD rig read-back
  // (rig_agc.or(agc)) until the next RX poll, so the clicked chip wouldn't light up. Sync from
  // the snapshot when it changes (confirms / corrects), exactly like the NR slider above.
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
  // Native Icom scope reference level, in tenths of a dB (−200..+200 = −20.0..+20.0 dB).
  const [scopeRefTenths, setScopeRefTenths] = useState(0)
  const changeScopeRef = (tenths: number) => {
    setScopeRefTenths(tenths)
    void setScopeRef(tenths)
  }
  const [keyed, setKeyed] = useState(false)
  // Bandscope span (audio-window zoom within the captured passband — this is
  // soundcard audio, not RF IQ, so "span" means which slice of the passband
  // fills the scope).
  const [span, setSpan] = useState<(typeof SPANS)[number]>(SPANS[0])
  const [rfSpan, setRfSpan] = useState<(typeof RF_SPANS)[number]>(RF_SPANS[0])
  // Live scope feed (reported by PhoneScope) — keeps the "RX audio" label honest when a
  // native RF panadapter is driving the scope (show the real RF span instead).
  const [scopeFeed, setScopeFeed] = useState<{ source: string; loHz: number; hiHz: number } | null>(
    null,
  )
  // True while a native RF panadapter (Flex/Icom CI-V) is actually streaming the scope. Drives
  // the whole panel's identity: when the rig's real RF spectrum is live we drop the audio-passband
  // framing (the "RX audio" label and the audio-Hz span chips) so the operator sees ONE unambiguous
  // display — the panadapter — instead of RF spectrum wrapped in audio-passband chrome.
  const nativeRf = scopeFeed != null && isRfScopeSource(scopeFeed.source)
  // True only when the rig's own Icom scope is streaming (span/ref are Icom CI-V commands; the
  // Flex panadapter has a different control path, so gate on 'civ' specifically, not any RF feed).
  const civScope = scopeFeed?.source === 'civ'
  // FlexRadio SmartSDR panadapter — its own span/ref command path (display pan set …).
  const flexScope = scopeFeed?.source === 'flex'
  const [flexRefDbm, setFlexRefDbm] = useState(-80)
  const changeFlexRef = (dbm: number) => {
    setFlexRefDbm(dbm)
    void setFlexPanRef(dbm)
      .then((s) => onSnap?.(s))
      .catch(() => {})
  }
  const [lock, setLock] = useState(false) // hands-free PTT (toggle instead of hold)
  const [recBusy, setRecBusy] = useState(false) // in-flight guard for the record toggle
  const [spotOpen, setSpotOpen] = useState(false) // spot-to-cluster popup
  // Wheel-to-tune over the bandscope, sharing the tuning strip's step selector.
  const [tuneStep, setTuneStep] = useState(100)
  const scopeRef = useRef<HTMLDivElement>(null)
  // Cockpit root: the scope-height splitter measures + writes its CSS var here.
  const cockpitRef = useRef<HTMLElement>(null)
  useWheelTune(scopeRef, {
    dialMhz: snap.radio.dialMhz,
    sideband: snap.radio.sideband || 'USB',
    enabled: snap.radio.catOk === true && !snap.radio.transmitting,
    stepHz: tuneStep,
    onSnap,
  })

  // AUTO sideband from the rig-mode policy — FM when the FM sub-mode is selected, else sideband
  // by band (LSB <10 MHz, USB above). The operator can override this transiently (below).
  const sidebandAuto =
    phoneMode?.toLowerCase() === 'fm' ? 'FM' : snap.radio.dialMhz < 10 ? 'LSB' : 'USB'
  // Transient operator override ("USB"/"LSB"/"FM") or null = AUTO. The COMMANDED mode (canonical
  // for TX/logging + what the rig is set to) is the override when set, else the band-auto sideband.
  const modeOverride = snap.radio.sidebandOverride ?? null
  const commandedMode = modeOverride ?? sidebandAuto
  const pickMode = (m: 'USB' | 'LSB' | 'FM' | null) =>
    void setSidebandOverride(m)
      .then((s) => onSnap?.(s))
      .catch(() => pushToast('Could not set mode', 'error'))
  // Whether the app can actually control the rig. Without CAT (VOX/serial PTT) the dial +
  // mode can't be set or read back — surface that so it's clear, not silently broken.
  const catOk = snap.radio.catOk === true

  // Click/drag tuning from the bandscope (Flex-style): clicks command immediately, a
  // drag coalesces to one CAT write per ~120 ms. PhoneScope does the signal-snap math
  // and reports the final dial; this hook just commands it.
  const onScopeTune = useScopeTune({
    sideband: commandedMode,
    enabled: catOk && !snap.radio.transmitting,
    onSnap,
  })

  // RX filter / passband width — the rig's read-back (null = unknown/default). The ± stepper
  // nudges it 100 Hz within a sane SSB/CW span, seeded from the current value or a 2.4 kHz default.
  const filterHz = snap.radio.filterWidthHz ?? null
  const bumpFilter = (deltaHz: number) => {
    const base = filterHz ?? 2400
    const next = Math.min(4000, Math.max(300, base + deltaHz))
    // Never let the clamp invert the direction ("wider" must not narrow at the rails).
    if ((deltaHz > 0 && next <= base) || (deltaHz < 0 && next >= base)) return
    void setFilterWidth(next)
      .then((s) => onSnap?.(s))
      .catch(() => pushToast('Could not set filter width', 'error'))
  }

  // Rig's actual mode read back over CAT (display-only). The app's `commandedMode` stays
  // canonical for TX/logging; this just flags when the rig's mode disagrees, so the badge
  // never silently lies.
  const rigMode = (snap.radio.rigMode ?? '').toUpperCase()
  // Collapse ONLY the FM variants (FMN/WFM → FM). Deliberately do NOT strip PKT/data suffixes:
  // in Phone a rig stuck in PKTUSB / DATA-U (rear-jack audio → dead mic) vs a commanded USB is a
  // REAL operational mismatch worth flagging, not a cosmetic naming variant.
  const rigFamily = /^W?FM/.test(rigMode) ? 'FM' : rigMode
  const modeMismatch = catOk && rigMode !== '' && rigFamily !== commandedMode ? rigMode : null

  // Manual split (casual DX "work up N"): the desired TX dial lives in the snapshot; a plain
  // retune clears it (backend). Offset is kHz off the RX dial; default +5, the common pileup.
  const [splitOffsetKhz, setSplitOffsetKhz] = useState(5)
  const splitTxMhz = snap.radio.splitTxMhz ?? null
  const splitOn = splitTxMhz != null
  // When split turns on externally (e.g. a pile-up spot programs it), sync the local offset
  // to the rig so the display + bumping start from the real value, not a stale default.
  const wasSplitOn = useRef(false)
  useEffect(() => {
    if (splitOn && !wasSplitOn.current && splitTxMhz != null) {
      setSplitOffsetKhz(Math.round((splitTxMhz - snap.radio.dialMhz) * 1000))
    }
    wasSplitOn.current = splitOn
  }, [splitOn, splitTxMhz, snap.radio.dialMhz])
  const applySplitTx = (offsetKhz: number) =>
    setSplit(snap.radio.dialMhz + offsetKhz / 1000)
      .then((s) => onSnap?.(s))
      .catch(() => pushToast('Could not set split', 'error'))
  const toggleSplit = () =>
    splitOn
      ? setSplit(null)
          .then((s) => onSnap?.(s))
          .catch(() => pushToast('Could not clear split', 'error'))
      : applySplitTx(splitOffsetKhz)
  // Accumulate on local state (functional updater) so rapid bumps that fire before the
  // IPC/onSnap round-trip don't all read the same stale value and collapse into one step.
  const bumpSplit = (delta: number) =>
    setSplitOffsetKhz((prev) => {
      const next = Math.max(-90, Math.min(90, prev + delta))
      if (splitOn) void applySplitTx(next)
      return next
    })

  // Live snapshot ref so the spacebar PTT handler (bound on `lock` changes, not every render)
  // reads the CURRENT TX-allowed privilege state through key() — not whatever existed when bound.
  const snapRef = useRef(snap)
  snapRef.current = snap
  const key = (on: boolean) => {
    // Don't key (or show ON-AIR) outside license privileges — the engine blocks it anyway.
    if (on && !snapRef.current.radio.txAllowed) {
      pushToast('TX locked — this frequency/mode is outside your license privileges', 'info', 3500)
      return
    }
    setKeyed(on)
    void setPtt(on)
  }
  const onPttDown = () => {
    if (lock) {
      key(!keyed) // hands-free: toggle
    } else {
      key(true)
    }
  }
  const onPttUp = () => {
    if (!lock) key(false)
  }
  const changePower = (pct: number) => {
    setPower(pct)
    void setRfPower(pct / 100)
  }

  // QSO recording (audio bridge): a session-level toggle driven by the snapshot (so the REC
  // badge survives nav + multi-window). Apply the returned snapshot immediately (no ~300 ms
  // poll lag) and guard re-entry so a rapid double-click can't double-fire.
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

  // Spacebar = push-to-talk (hold), unless typing in a field.
  useEffect(() => {
    const isField = (t: EventTarget | null) =>
      t instanceof HTMLElement && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA')
    const down = (e: KeyboardEvent) => {
      if (e.code === 'Space' && !e.repeat && !isField(e.target) && !lock) {
        e.preventDefault()
        key(true)
      }
    }
    const up = (e: KeyboardEvent) => {
      if (e.code === 'Space' && !isField(e.target) && !lock) {
        e.preventDefault()
        key(false)
      }
    }
    window.addEventListener('keydown', down)
    window.addEventListener('keyup', up)
    return () => {
      window.removeEventListener('keydown', down)
      window.removeEventListener('keyup', up)
      void setPtt(false) // safety: never leave the rig keyed on unmount
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [lock])

  // Field Day exchange the operator reads aloud (and the string to record into a voice-keyer
  // slot). Class + Section from the active ruleset; empty until FD setup fills them in.
  const fdExchange = fieldDay ? `${fieldDay.myClass} ${fieldDay.mySection}`.trim() : ''

  return (
    <main className="layout single phone-cockpit" ref={cockpitRef}>
      <div className="ph-bar">
        <div className="ph-mode-pick" role="group" aria-label="Phone mode">
          {(['AUTO', 'USB', 'LSB', 'FM'] as const).map((m) => {
            const active = m === 'AUTO' ? modeOverride === null : modeOverride === m
            return (
              <button
                key={m}
                type="button"
                className={`ph-mode-btn${active ? ' active' : ''}`}
                aria-pressed={active}
                disabled={!catOk}
                title={
                  m === 'AUTO'
                    ? `AUTO — sideband by band (now ${sidebandAuto}); a band change re-asserts this`
                    : `Force ${m} until you change bands`
                }
                onClick={() => pickMode(m === 'AUTO' ? null : m)}
              >
                {m === 'AUTO' ? `AUTO·${sidebandAuto}` : m}
              </button>
            )
          })}
        </div>
        {modeMismatch && (
          <span
            className="ph-mode-mismatch"
            title={`Your rig is on ${modeMismatch}, but Phone is set to ${commandedMode}. Logging and TX use ${commandedMode} — turn the rig's mode knob (or re-pick the band) to match.`}
          >
            rig: {modeMismatch}
          </span>
        )}
        <TuningStrip snap={snap} onSnap={onSnap} step={tuneStep} onStep={setTuneStep} />
        <BandPicker snap={snap} mode="phone" onSnap={onSnap} />
        {catOk && (
          <div className={`ph-split ${splitOn ? 'on' : ''}`}>
            <button
              className="ph-split-toggle"
              onClick={toggleSplit}
              title={
                splitOn
                  ? `Split ON — TX ${splitTxMhz?.toFixed(3)} MHz. Click for simplex.`
                  : 'Work split — TX off your RX frequency (e.g. up 5)'
              }
            >
              SPLIT
            </button>
            <button className="ph-split-step" onClick={() => bumpSplit(-1)} title="TX 1 kHz lower">
              −
            </button>
            <span className="ph-split-amt mono" title="TX offset from your RX dial (kHz)">
              {splitOffsetKhz >= 0 ? `+${splitOffsetKhz}` : `${splitOffsetKhz}`}
            </span>
            <button className="ph-split-step" onClick={() => bumpSplit(1)} title="TX 1 kHz higher">
              +
            </button>
          </div>
        )}
        {!catOk && (
          <span
            className="ph-nocat"
            title={
              snap.radio.catDetail ||
              'No CAT link — set a rigctld/CAT rig in Settings so the app can switch the mode and follow the dial. On VOX/RTS-DTR PTT the rig has no command channel.'
            }
          >
            ⚠ no rig control
          </span>
        )}
        <label className="ph-power" title="RF output power">
          <span>Power</span>
          <input
            type="range"
            min={0}
            max={100}
            value={power}
            onChange={(e) => changePower(Number(e.target.value))}
            onPointerDown={() => {
              dragging.current = true
            }}
            onPointerUp={() => {
              dragging.current = false
            }}
            aria-label="RF power"
          />
          <span className="ph-power-val">{power}%</span>
        </label>
        {snap.radio.micGain != null && (
          <label className="ph-power" title="Microphone gain — raise it until SSB peaks tickle the ALC zone">
            <span>Mic</span>
            <input
              type="range"
              min={0}
              max={100}
              value={mic}
              onChange={(e) => changeMic(Number(e.target.value))}
              onPointerDown={() => {
                micDragging.current = true
              }}
              onPointerUp={() => {
                micDragging.current = false
              }}
              aria-label="Mic gain"
            />
            <span className="ph-power-val">{mic}%</span>
          </label>
        )}
        {catOk && commandedMode !== 'FM' && (
          <div className="ph-filter" title="RX filter / passband width (CAT)">
            <span className="ph-filter-lbl">BW</span>
            <button
              type="button"
              className="ph-filter-step"
              onClick={() => bumpFilter(-100)}
              title="Narrower (−100 Hz)"
            >
              −
            </button>
            <span className="ph-filter-val mono">
              {filterHz ? `${(filterHz / 1000).toFixed(1)}k` : '—'}
            </span>
            <button
              type="button"
              className="ph-filter-step"
              onClick={() => bumpFilter(100)}
              title="Wider (+100 Hz)"
            >
              +
            </button>
          </div>
        )}
        <span className="ph-spacer" />
        <MemoryBank
          dialMhz={snap.radio.dialMhz}
          mode={commandedMode}
          onRecall={(freqMhz, mode) => {
            // The Phone rig-mode policy derives the commanded mode from the
            // phone sub-mode (fm vs ssb→band-sideband), NOT from the sideband
            // arg — so a saved FM (or SSB) channel only round-trips if we first
            // switch phone_mode to match, otherwise the rig lands on the wrong
            // mode. Flip phone_mode only when it actually differs, then retune.
            void (async () => {
              const wantFm = mode.toUpperCase() === 'FM'
              if (wantFm !== (phoneMode?.toLowerCase() === 'fm')) {
                const s = await getSettings()
                await setSettings({ ...s, phoneMode: wantFm ? 'fm' : 'ssb' })
              }
              // A recalled memory carries its own mode — drop any manual override (even same-band)
              // so the saved channel's mode wins instead of a stale forced sideband.
              await setSidebandOverride(null)
              const snap = await setFrequency(freqMhz, bandLabelForMhz(freqMhz), mode)
              onSnap?.(snap)
            })()
          }}
        />
        <RotorStrip />
        <button
          type="button"
          className="ph-rec"
          onClick={() => setSpotOpen(true)}
          title="Spot a callsign to the DX cluster (opens a popup — call, frequency, comment)"
        >
          📢 Spot
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
        <label className="ph-rxmeter" title="RX audio level">
          <span>RX</span>
          <LevelMeter value={snap.radio.rxLevel} label="RX audio level" variant="compact" />
        </label>
        <span className={`ph-tx ${snap.radio.transmitting ? 'on' : ''}`}>
          {snap.radio.transmitting ? '▲ TX' : snap.radio.txEnabled ? '▼ RX' : '■ TX off'}
        </span>
      </div>

      <section className="ph-scope-panel">
        <div className="ph-scope-head">
          {(() => {
            // Honest per feed: soundcard FFT = the demodulated RX audio; a native
            // panadapter = the real RF spectrum, so name it a panadapter and show its span.
            const rf = nativeRf ? scopeFeed : null
            return (
              <span
                className="ph-scope-title"
                title={
                  rf
                    ? 'Native RF panadapter — the real RF spectrum around your dial, not the demodulated audio passband.'
                    : 'Receiver AUDIO spectrum (200–2900 Hz of the demodulated passband) — not a band-wide RF panadapter, so a voice fills the passband rather than sliding across it as you tune.'
                }
              >
                {rf ? 'RF Panadapter' : 'Passband'}{' '}
                <span className="ph-scope-sub">
                  {rf
                    ? `· ${(rf.loHz / 1e6).toFixed(4)}–${(rf.hiHz / 1e6).toFixed(4)} MHz`
                    : '· RX audio'}
                </span>
              </span>
            )
          })()}
          <span className="ph-scope-head-label">Colors</span>
          <PalettePicker />
        </div>
        <div className="ph-scope-wrap" ref={scopeRef} title="Scroll here to tune the VFO">
          {nativeRf ? (
            // Native RF panadapter: RF-width zoom around the dial (not audio-passband slices).
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
          ) : (
            <div className="ph-span" role="group" aria-label="Bandscope span">
              {SPANS.map((sp) => (
                <button
                  key={sp.label}
                  type="button"
                  className={`theme-chip${span.label === sp.label ? ' active' : ''}`}
                  aria-pressed={span.label === sp.label}
                  title={sp.title}
                  onClick={() => setSpan(sp)}
                >
                  {sp.label}
                </button>
              ))}
            </div>
          )}
          <PhoneScope
            transmitting={snap.radio.transmitting}
            theme={theme}
            smeterDb={snap.radio.smeterDb}
            viewLoHz={nativeRf ? rfSpan.lo : span.lo}
            viewHiHz={nativeRf ? rfSpan.hi : span.hi}
            sideband={commandedMode}
            dialHz={snap.radio.dialMhz > 0 ? Math.round(snap.radio.dialMhz * 1e6) : null}
            onFeed={(source, loHz, hiHz) => setScopeFeed({ source, loHz, hiHz })}
            onTune={onScopeTune}
            filterWidthHz={filterHz ?? 2400}
            interactive={catOk && !snap.radio.transmitting && snap.radio.dialMhz > 0}
          />
        </div>
      </section>
      <Splitter
        axis="y"
        varName="--ph-scope-h"
        target={cockpitRef}
        storageKey="nexus.split.phone.scope"
        minPx={100}
        maxPx={420}
        defaultPct={22}
        label="scope height"
      />

      {/* Rig scope controls (native Icom CI-V only) — drive the RADIO's real panadapter: span
          changes the hardware sweep width, ref sets weak-signal visibility. Distinct from the
          view-zoom chips on the scope itself, which only zoom what's already streamed. */}
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

      {/* FlexRadio SmartSDR panadapter controls — command the Flex pan's real bandwidth + ref. */}
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

      {/* Transmit meters (SWR/ALC/Po/COMP) — appear only while keyed, where the S-meter sat. */}
      <TxMeters radio={snap.radio} />

      {(() => {
          // Only funcs the rig actually reports (non-null) render — capability-gated, no dead buttons.
          const supported = DSP_FUNCS.filter((f) => snap.radio[f.key] != null)
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

      {/* RX DSP levels — NR level slider + AGC speed, each shown only when the rig reports it. */}
      {(snap.radio.nrLevel != null || snap.radio.agc != null) && (
        <div className="ph-dsp-levels" role="group" aria-label="RX DSP levels">
          {snap.radio.nrLevel != null && (
            <label className="ph-dsplev" title="Noise-reduction depth — raise until the noise floor drops, back off if audio gets watery">
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
            <div className="ph-agc" role="group" aria-label="AGC speed" title="AGC time constant">
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

      {onWorkSpot && (
        <BandStrip
          band={snap.radio.band}
          dialMhz={snap.radio.dialMhz}
          txAllowed={snap.radio.txAllowed}
          phoneSegLo={snap.radio.phoneSegLo}
          phoneSegHi={snap.radio.phoneSegHi}
          spots={spots ?? []}
          needByCall={needByCall}
          typeByCall={typeByCall}
          onWorkSpot={onWorkSpot}
          onPopOut={() => void openPanelWindow('bandmapPhone')}
        />
      )}

      <div className="ph-ptt-row">
        {fdExchange && (
          <span
            className="ph-fd-give"
            title="Field Day exchange — read this to the station you're working (your class + section)."
          >
            <span className="ph-fd-give-lbl">Give</span>
            <span className="ph-fd-give-exch mono">{fdExchange}</span>
          </span>
        )}
        <button
          type="button"
          className={`ph-ptt${keyed ? ' keyed' : ''}`}
          onPointerDown={onPttDown}
          onPointerUp={onPttUp}
          onPointerLeave={onPttUp}
          disabled={!snap.radio.txAllowed}
          title={
            snap.radio.txAllowed
              ? "Hold to talk (or Space). Toggle 'Lock' for hands-free."
              : 'TX locked — outside your license privileges (pick a band, or change your license in Settings)'
          }
        >
          {!snap.radio.txAllowed ? '🔒 TX LOCKED' : keyed ? 'ON AIR — release to stop' : 'PUSH TO TALK'}
        </button>
        <label className="ph-lock" title="Hands-free: click PTT once to key, again to unkey">
          <input type="checkbox" checked={lock} onChange={(e) => setLock(e.target.checked)} />
          <span>Lock</span>
        </label>
        <span className="ph-ptt-hint">Hold the button or the Space bar · you talk on the rig's mic</span>
        <div className="ph-tx-utils">
          <button
            type="button"
            className={`ph-tune${snap.radio.tuning ? ' keyed' : ''}`}
            aria-pressed={snap.radio.tuning}
            onClick={() => void setTune(!snap.radio.tuning).then((s) => onSnap?.(s))}
            disabled={!snap.radio.txAllowed}
            title="Key a steady carrier to tune an ATU/amp (auto-stops on the tune watchdog). Click again to stop."
          >
            {snap.radio.tuning ? 'TUNING…' : 'Tune'}
          </button>
          <button
            type="button"
            className="ph-stoptx"
            onClick={() => void haltTx()}
            title="Stop transmitting immediately — unkey PTT and drop the tune carrier"
          >
            Stop TX
          </button>
        </div>
      </div>

      <VoiceKeyer txEnabled={snap.radio.txEnabled} keyed={keyed} fdExchange={fdExchange} />

      <LogEntry
        snap={snap}
        mode={commandedMode === 'FM' ? 'FM' : 'SSB'}
        defaultRst="59"
        pendingWork={pendingWork}
        onConsumeWork={onConsumeWork}
        fieldDay={fieldDay}
        fdMode="PH"
      />
      <SpotDialog
        open={spotOpen}
        onClose={() => setSpotOpen(false)}
        initialCall=""
        freqMhz={snap.radio.dialMhz}
        defaultComment={commandedMode}
      />
    </main>
  )
}
