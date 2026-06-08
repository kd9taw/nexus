import { useEffect, useState } from 'react'
import type { BandChannel, LinkState, RadioStatus, Tier } from '../types'
import type { Theme } from '../useTheme'
import type { Layout } from '../useLayout'
import { ThemeSwitcher } from './ThemeSwitcher'
import { FrequencyControl } from './FrequencyControl'
import { StatusLane } from './StatusLane'
import { LevelMeter } from './LevelMeter'

interface Props {
  mycall: string
  mygrid: string
  radio: RadioStatus
  link: LinkState
  bandPlan: BandChannel[]
  onSetFrequency: (dialMhz: number, band: string, mode: string) => void
  onSetTxEnabled: (enabled: boolean) => void
  onSetTune: (on: boolean) => void
  onHaltTx: () => void
  onSetTxEven: (even: boolean) => void
  onSetHoldTxFreq: (on: boolean) => void
  wfLayout: Layout
  onWfLayoutChange: (l: Layout) => void
  tier: Tier
  onTierChange: (t: Tier) => void
  theme: Theme
  onThemeChange: (t: Theme) => void
}

// The robust tier is DX1 — a non-coherent, fading-resilient 15 s mode that
// holds up where FT1 (and FT8) collapse under multipath/Doppler. FT8 itself is
// a separate Phase-2 addition (its decode pipeline isn't wired yet).

function dtLabel(dtSec: number): string {
  const v = Math.round(dtSec * 10) / 10
  return `DT ${v > 0 ? '+' : ''}${v.toFixed(1)}s`
}

/** Color class for the NTP clock offset: ok ≤0.3 s, warn ≤1 s, else bad. */
function clockClass(ms: number): string {
  const a = Math.abs(ms)
  return a <= 300 ? 'ok' : a <= 1000 ? 'warn' : 'bad'
}

/** Format the clock offset as a signed seconds value, e.g. "+0.32s". */
function clockLabel(ms: number): string {
  const s = ms / 1000
  return `${s > 0 ? '+' : ''}${s.toFixed(2)}s`
}

/** Live UTC clock (HH:MM:SS), ticking once a second. */
function UtcClock() {
  const [now, setNow] = useState(() => new Date())
  useEffect(() => {
    const id = window.setInterval(() => setNow(new Date()), 1000)
    return () => window.clearInterval(id)
  }, [])
  const p = (n: number) => String(n).padStart(2, '0')
  const hhmmss = `${p(now.getUTCHours())}:${p(now.getUTCMinutes())}:${p(now.getUTCSeconds())}`
  return (
    <div className="utc-clock" title="UTC time">
      <span className="utc-time">{hhmmss}</span>
      <span className="utc-label">UTC</span>
    </div>
  )
}

export function TopBar({
  mycall,
  mygrid,
  radio,
  link,
  bandPlan,
  onSetFrequency,
  onSetTxEnabled,
  onSetTune,
  onHaltTx,
  onSetTxEven,
  onSetHoldTxFreq,
  wfLayout,
  onWfLayoutChange,
  tier,
  onTierChange,
  theme,
  onThemeChange,
}: Props) {
  const countdown = (radio.nextSlotMs / 1000).toFixed(1)
  return (
    <header className="topbar">
      <div className="topbar-group brand">
        <span className="logo">Nexus</span>
        <span className="mycall">
          {mycall}
          <span className="mygrid">{mygrid}</span>
        </span>
      </div>

      <div className="topbar-group radio-readout">
        <FrequencyControl
          channels={bandPlan}
          dialMhz={radio.dialMhz}
          band={radio.band}
          mode={radio.sideband}
          variant="compact"
          onSet={onSetFrequency}
        />
      </div>

      <div className="topbar-group txrx">
        <span className={`txrx-indicator ${radio.transmitting ? 'tx' : 'rx'}`}>
          {radio.transmitting ? 'TX' : 'RX'}
        </span>

        <div className="rx-level" title={`RX audio level ${Math.round(radio.rxLevel * 100)}%`}>
          <span className="rx-level-label">RX</span>
          <LevelMeter value={radio.rxLevel} label="RX audio level" variant="compact" />
        </div>

        <div className="op-controls" role="group" aria-label="Transmit controls">
          <button
            type="button"
            className={`op-btn monitor${radio.txEnabled ? ' on' : ''}`}
            aria-pressed={radio.txEnabled}
            onClick={() => onSetTxEnabled(!radio.txEnabled)}
            title={
              radio.txEnabled
                ? 'Transmit ENABLED — your queued message will go out. Click to disable transmit (receive keeps decoding either way).'
                : 'Transmit DISABLED — receive keeps decoding. Click to enable transmit (WSJT-X "Enable Tx").'
            }
          >
            {radio.txEnabled ? 'TX On' : 'TX Off'}
          </button>
          <button
            type="button"
            className={`op-btn tune${radio.tuning ? ' keyed' : ''}`}
            aria-pressed={radio.tuning}
            onClick={() => onSetTune(!radio.tuning)}
            title="Key a tune carrier"
          >
            Tune
          </button>
          <button
            type="button"
            className="op-btn stop"
            onClick={onHaltTx}
            title="Stop transmitting immediately"
          >
            Stop TX
          </button>
          <button
            type="button"
            className={`op-btn hold${radio.holdTxFreq ? ' on' : ''}`}
            aria-pressed={radio.holdTxFreq}
            onClick={() => onSetHoldTxFreq(!radio.holdTxFreq)}
            title="Hold Tx Freq: keep your TX offset fixed when you click the waterfall to set RX"
          >
            Hold Tx
          </button>
        </div>

        {radio.txWatchdog && (
          <span className="watchdog-chip" role="alert" title="Transmit was auto-halted by the TX watchdog. Click TX On to re-enable.">
            ⚠ TX watchdog — auto-halted
          </span>
        )}

        <StatusLane />
        <div className="slot-clock" title="Time to next slot">
          <span className="slot-count">{countdown}s</span>
          <span className="slot-label">next slot</span>
        </div>
        <UtcClock />
        {radio.clockOffsetMs != null ? (
          <span
            className={`timesync ${clockClass(radio.clockOffsetMs)}`}
            title={`PC clock is ${clockLabel(radio.clockOffsetMs)} vs UTC (NTP). FT1/DX1 need it within ~0.5 s — sync via NTP / time.is (off-grid: GPS).`}
          >
            <span className="dot" />
            clock {clockLabel(radio.clockOffsetMs)}
          </span>
        ) : (
          <span
            className={`timesync ${radio.timeSyncOk ? 'ok' : 'bad'}`}
            title={
              radio.timeSyncOk
                ? 'Time sync OK (from decode timing)'
                : 'Decodes land far off the slot boundary — sync your PC clock (NTP / time.is; off-grid: GPS).'
            }
          >
            <span className="dot" />
            {radio.timeSyncOk ? 'Sync' : 'No Sync'}
          </span>
        )}
        <span
          className={`dt-readout${Math.abs(link.dtSec) > 0.5 ? ' bad' : ''}`}
          title="Decode time offset (how far heard signals land from the slot boundary)"
        >
          {dtLabel(link.dtSec)}
        </span>
      </div>

      <div className="topbar-group tier-toggle" role="group" aria-label="Link tier">
        <button
          type="button"
          className={`tier-btn${tier === 'FT1' ? ' active' : ''}`}
          aria-pressed={tier === 'FT1'}
          onClick={() => onTierChange('FT1')}
          title="Fast conversational tier"
        >
          Fast <small>FT1</small>
        </button>
        <button
          type="button"
          className={`tier-btn${tier === 'DX1' ? ' active' : ''}`}
          aria-pressed={tier === 'DX1'}
          onClick={() => onTierChange('DX1')}
          title="Robust non-coherent tier — fading-resilient (15 s)"
        >
          Robust <small>DX1</small>
        </button>
        <button
          type="button"
          className={`tier-btn${tier === 'FT8' ? ' active' : ''}`}
          aria-pressed={tier === 'FT8'}
          onClick={() => onTierChange('FT8')}
          title="Standard WSJT-X FT8 (15 s)"
        >
          <small>FT8</small>
        </button>
        <button
          type="button"
          className={`tier-btn${tier === 'FT4' ? ' active' : ''}`}
          aria-pressed={tier === 'FT4'}
          onClick={() => onTierChange('FT4')}
          title="Standard WSJT-X FT4 (7.5 s)"
        >
          <small>FT4</small>
        </button>
      </div>

      <div className="topbar-group tier-toggle tx-period" role="group" aria-label="Transmit period">
        <button
          type="button"
          className={`tier-btn${radio.txEven ? ' active' : ''}`}
          aria-pressed={radio.txEven}
          onClick={() => onSetTxEven(true)}
          title="Transmit in the even (1st) T/R slots — the station you work must be Tx 2nd"
        >
          Tx 1st <small>even</small>
        </button>
        <button
          type="button"
          className={`tier-btn${!radio.txEven ? ' active' : ''}`}
          aria-pressed={!radio.txEven}
          onClick={() => onSetTxEven(false)}
          title="Transmit in the odd (2nd) T/R slots — the station you work must be Tx 1st"
        >
          Tx 2nd <small>odd</small>
        </button>
      </div>

      <div className="topbar-group tier-toggle wf-layout" role="group" aria-label="Waterfall position">
        <button
          type="button"
          className={`tier-btn${wfLayout === 'right' ? ' active' : ''}`}
          aria-pressed={wfLayout === 'right'}
          onClick={() => onWfLayoutChange('right')}
          title="Waterfall on the right rail"
        >
          WF <small>right</small>
        </button>
        <button
          type="button"
          className={`tier-btn${wfLayout === 'top' ? ' active' : ''}`}
          aria-pressed={wfLayout === 'top'}
          onClick={() => onWfLayoutChange('top')}
          title="Waterfall as a strip across the top"
        >
          WF <small>top</small>
        </button>
      </div>

      <div className="topbar-group">
        <ThemeSwitcher theme={theme} onChange={onThemeChange} />
      </div>
    </header>
  )
}
