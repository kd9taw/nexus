// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen, fireEvent, waitFor, cleanup } from '@testing-library/react'
import { RttyCockpit, confidenceRuns } from './RttyCockpit'
import * as api from '../api'
import * as toast from '../toast'
import type { AppSnapshot, RttyState } from '../types'

vi.mock('../api', () => ({
  getRttyState: vi.fn(),
  rttyArm: vi.fn(),
  getLicensedBandPlan: vi.fn(),
  rttySend: vi.fn(),
  rttyStop: vi.fn(),
  rttyClear: vi.fn(),
  rttyAfcReset: vi.fn(),
  haltTx: vi.fn(),
}))
vi.mock('../toast', () => ({
  pushToast: vi.fn(),
  // Pass-through like the real one: run the action, null on failure.
  withErrorToast: vi.fn(async (action: () => Promise<unknown>) => {
    try {
      return await action()
    } catch {
      return null
    }
  }),
}))

const getRttyState = api.getRttyState as ReturnType<typeof vi.fn>
const rttyArm = api.rttyArm as ReturnType<typeof vi.fn>
const getLicensedBandPlan = api.getLicensedBandPlan as ReturnType<typeof vi.fn>
const rttySend = api.rttySend as ReturnType<typeof vi.fn>
const rttyStop = api.rttyStop as ReturnType<typeof vi.fn>
const haltTx = api.haltTx as ReturnType<typeof vi.fn>
const pushToast = toast.pushToast as ReturnType<typeof vi.fn>

const snap = {
  mycall: 'KD9TAW',
  radio: {
    dialMhz: 14.08,
    band: '20m',
    catOk: true,
    sideband: 'USB',
    transmitting: false,
    txEnabled: true,
    tuning: false,
    txAllowed: true,
  },
} as unknown as AppSnapshot

const IDLE: RttyState = {
  armed: false,
  afcHz: 0,
  afcLocked: false,
  text: '',
  charConf: [],
  baud: 45.45,
  shiftHz: 170,
  backend: 'afsk',
  sending: false,
  keyerError: null,
}

beforeEach(() => {
  getRttyState.mockReset().mockResolvedValue(IDLE)
  rttyArm.mockReset().mockResolvedValue({ ...IDLE, armed: true })
  getLicensedBandPlan.mockReset().mockResolvedValue([])
  rttySend.mockReset().mockResolvedValue({ ...IDLE, sending: true })
  rttyStop.mockReset().mockResolvedValue(IDLE)
  haltTx.mockReset().mockResolvedValue(snap)
  pushToast.mockReset()
})
afterEach(cleanup)

describe('RttyCockpit RX wiring', () => {
  it('renders without a snapshot (stream + macros + compose, no header)', async () => {
    render(<RttyCockpit snap={null} />)
    expect(screen.getByText('Arm RX to decode RTTY from the receive audio')).toBeTruthy()
    // No snapshot → no CockpitHeader (it needs live radio state).
    expect(document.querySelector('.cockpit-header')).toBeNull()
    expect(screen.getByLabelText('RTTY compose')).toBeTruthy()
    await waitFor(() => expect(getRttyState).toHaveBeenCalled())
  })

  it('renders the mode badge + keying-backend pill with a snapshot', async () => {
    render(<RttyCockpit snap={snap} />)
    await screen.findByText('RTTY 45.45 · 170 Hz')
    expect(screen.getByText('AFSK')).toBeTruthy()
    // No onSetFrequency handler → the display-only band pill.
    expect(screen.getByText('20m')).toBeTruthy()
  })

  it('offers the licensed RTTY band plan and QSYs through onSetFrequency', async () => {
    getLicensedBandPlan.mockResolvedValue([
      {
        band: '20m',
        group: 'HF',
        dialMhz: 14.083,
        mode: 'LSB',
        label: '20 m · RTTY',
        note: 'the 14.080–14.090 RTTY window',
      },
    ])
    const onSetFrequency = vi.fn()
    render(<RttyCockpit snap={snap} onSetFrequency={onSetFrequency} />)
    expect(getLicensedBandPlan).toHaveBeenCalledWith('rtty')
    const select = (await screen.findByLabelText('Band channel preset')) as HTMLSelectElement
    await waitFor(() => expect(select.querySelectorAll('option').length).toBeGreaterThan(1))
    fireEvent.change(select, { target: { value: '20m' } })
    // Lands on the watering hole with the channel's own sideband (RTTY = LSB).
    expect(onSetFrequency).toHaveBeenCalledWith(14.083, '20m', 'LSB')
  })

  it('polls the decoder and renders confidence-faded text + the locked AFC pill', async () => {
    getRttyState.mockResolvedValue({
      ...IDLE,
      armed: true,
      afcHz: 12.4,
      afcLocked: true,
      text: 'CQ TEST',
      // "CQ TE" solid, "ST" low-confidence → faint tail run.
      charConf: [95, 95, 95, 90, 90, 20, 20],
    })
    render(<RttyCockpit snap={snap} />)
    await screen.findByText('RX armed')
    expect(screen.getByText('+12 Hz 🔒')).toBeTruthy()
    const faint = screen.getByText('ST')
    expect(faint.style.opacity).toBe('0.3')
    expect(screen.getByText('CQ TE').style.opacity).toBe('')
  })

  it('shows the unlocked AFC offset plain (no lock glyph)', async () => {
    getRttyState.mockResolvedValue({ ...IDLE, armed: true, afcHz: -8.2 })
    render(<RttyCockpit snap={snap} />)
    await screen.findByText('-8 Hz')
    expect(screen.queryByText(/🔒/)).toBeNull()
  })

  it('arms the RX decoder through rtty_arm and reflects the returned state', async () => {
    render(<RttyCockpit snap={snap} />)
    const arm = await screen.findByText('Arm RX')
    fireEvent.click(arm)
    expect(rttyArm).toHaveBeenCalledWith(true)
    await screen.findByText('RX armed')
  })

  it('does not poll while inactive (hidden keep-alive host)', () => {
    render(<RttyCockpit snap={snap} active={false} />)
    expect(getRttyState).not.toHaveBeenCalled()
  })
})

describe('RttyCockpit TX wiring', () => {
  it('sends the CQ macro with {MYCALL} expanded — an explicit operator action', async () => {
    render(<RttyCockpit snap={snap} />)
    fireEvent.click(screen.getByText('CQ'))
    await waitFor(() =>
      expect(rttySend).toHaveBeenCalledWith('CQ CQ CQ DE KD9TAW KD9TAW K'),
    )
  })

  it('refuses a {CALL} macro until their call is entered — then expands it', async () => {
    render(<RttyCockpit snap={snap} />)
    fireEvent.click(screen.getByText('Answer'))
    expect(rttySend).not.toHaveBeenCalled()
    expect(pushToast).toHaveBeenCalled()
    fireEvent.change(screen.getByLabelText('Worked station callsign (the {CALL} macro token)'), {
      target: { value: 'w1abc' },
    })
    fireEvent.click(screen.getByText('Answer'))
    await waitFor(() =>
      expect(rttySend).toHaveBeenCalledWith('W1ABC DE KD9TAW KD9TAW K'),
    )
  })

  it('sends typed compose text on Send and clears the input', async () => {
    render(<RttyCockpit snap={snap} />)
    const input = screen.getByLabelText('RTTY compose') as HTMLInputElement
    fireEvent.change(input, { target: { value: 'tu 73' } })
    fireEvent.click(screen.getByText('Send'))
    await waitFor(() => expect(rttySend).toHaveBeenCalledWith('tu 73'))
    expect(input.value).toBe('')
  })

  it('refuses to key outside license privileges (surfaced up front)', () => {
    const locked = {
      ...snap,
      radio: { ...snap.radio, txAllowed: false },
    } as unknown as AppSnapshot
    render(<RttyCockpit snap={locked} />)
    fireEvent.click(screen.getByText('CQ'))
    expect(rttySend).not.toHaveBeenCalled()
    expect(pushToast).toHaveBeenCalledWith(
      'TX locked — this frequency is outside your license privileges',
      'info',
      3500,
    )
  })

  it('shows the sending pill while an over is on the air and Stop aborts + halts', async () => {
    getRttyState.mockResolvedValue({ ...IDLE, sending: true })
    render(<RttyCockpit snap={snap} />)
    await screen.findByText('TX ▲')
    const stop = screen.getByText('Stop').closest('button') as HTMLButtonElement
    expect(stop.disabled).toBe(false)
    fireEvent.click(stop)
    expect(rttyStop).toHaveBeenCalled()
    expect(haltTx).toHaveBeenCalled()
  })

  it('surfaces a keyer failure from the poll', async () => {
    getRttyState.mockResolvedValue({
      ...IDLE,
      keyerError: 'FSK keyline: couldn’t open COM7.',
    })
    render(<RttyCockpit snap={snap} />)
    await screen.findByRole('alert')
    expect(screen.getByRole('alert').textContent).toContain('FSK keyline')
  })
})

describe('confidenceRuns', () => {
  it('groups equal-confidence chars into runs and fades the low ones', () => {
    expect(confidenceRuns('ABCD', [90, 90, 20, 20])).toEqual([
      { text: 'AB', opacity: 1 },
      { text: 'CD', opacity: 0.3 },
    ])
  })

  it('treats missing confidence as solid — decoded text is never hidden', () => {
    expect(confidenceRuns('AB', [])).toEqual([{ text: 'AB', opacity: 1 }])
  })
})
