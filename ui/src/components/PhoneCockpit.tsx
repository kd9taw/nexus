import { useEffect, useState, useRef } from 'react'
import type { AppSnapshot, FieldDayStatus, SpotRow } from '../types'
import { PhoneScope } from './PhoneScope'
import { BandStrip } from './BandStrip'
import { PalettePicker } from './PalettePicker'
import { BandPicker } from './BandPicker'
import { VoiceKeyer } from './VoiceKeyer'
import { LevelMeter } from './LevelMeter'
import { LogEntry } from './LogEntry'
import { setPtt, setRfPower, startQsoRecording, stopQsoRecording } from '../api'
import { pushToast } from '../toast'
import { RotorStrip } from './RotorStrip'
import { MemoryBank } from './MemoryBank'
import { setFrequency, getSettings, setSettings, setSplit, setRigFunc, setSidebandOverride, setFilterWidth, openPanelWindow } from '../api'
import { bandLabelForMhz } from '../band'

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
  /** Work a spotted station from the band-strip (QSY to its freq + prefill the log). */
  onWorkSpot?: (s: SpotRow) => void
}

/**
 * Phone (voice) operating cockpit — casual/ragchew. The voice is the signal, so the
 * app does rig control + PTT + logging (you talk into the rig's mic; the live-mic
 * audio bridge + voice keyer land in P3-b/c). Entering forces USB/LSB by band (the
 * rig-mode keystone, wired in App). See `tasks/specs/phone-operating.md`.
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
  { label: 'Full', lo: 200, hi: 2900, title: 'Whole audio passband (200–2900 Hz)' },
  { label: 'Voice', lo: 300, hi: 2700, title: 'Voice energy (300–2700 Hz)' },
  { label: 'Low', lo: 200, hi: 1500, title: 'Lower half — zoomed (200–1500 Hz)' },
  { label: 'High', lo: 1500, hi: 2900, title: 'Upper half — zoomed (1500–2900 Hz)' },
] as const

export function PhoneCockpit({ snap, theme, pendingWork, onConsumeWork, onSnap, fieldDay, phoneMode, spots, onWorkSpot }: Props) {
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
  const [keyed, setKeyed] = useState(false)
  // Bandscope span (audio-window zoom within the captured passband — this is
  // soundcard audio, not RF IQ, so "span" means which slice of the passband
  // fills the scope).
  const [span, setSpan] = useState<(typeof SPANS)[number]>(SPANS[0])
  const [lock, setLock] = useState(false) // hands-free PTT (toggle instead of hold)
  const [recBusy, setRecBusy] = useState(false) // in-flight guard for the record toggle

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

  const key = (on: boolean) => {
    // Don't key (or show ON-AIR) outside license privileges — the engine blocks it anyway.
    if (on && !snap.radio.txAllowed) {
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

  return (
    <main className="layout single phone-cockpit">
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
        <span className="ph-freq mono">
          {snap.radio.dialMhz.toFixed(3)} MHz · {snap.radio.band}
        </span>
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
          <span
            className="ph-scope-title"
            title="A ~3 kHz slice of the receiver audio passband — not a band-wide panadapter"
          >
            Passband
          </span>
          <span className="ph-scope-head-label">Colors</span>
          <PalettePicker />
        </div>
        <div className="ph-scope-wrap">
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
          <PhoneScope
            transmitting={snap.radio.transmitting}
            theme={theme}
            smeterDb={snap.radio.smeterDb}
            viewLoHz={span.lo}
            viewHiHz={span.hi}
          />
        </div>
      </section>

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

      {onWorkSpot && (
        <BandStrip
          band={snap.radio.band}
          dialMhz={snap.radio.dialMhz}
          txAllowed={snap.radio.txAllowed}
          phoneSegLo={snap.radio.phoneSegLo}
          phoneSegHi={snap.radio.phoneSegHi}
          spots={spots ?? []}
          onWorkSpot={onWorkSpot}
          onPopOut={() => void openPanelWindow('bandmapPhone')}
        />
      )}

      <div className="ph-ptt-row">
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
      </div>

      <VoiceKeyer txEnabled={snap.radio.txEnabled} keyed={keyed} />

      <LogEntry
        snap={snap}
        mode={commandedMode === 'FM' ? 'FM' : 'SSB'}
        defaultRst="59"
        pendingWork={pendingWork}
        onConsumeWork={onConsumeWork}
        fieldDay={fieldDay}
        fdMode="PH"
      />
    </main>
  )
}
