import { describe, it, expect, vi, beforeEach } from 'vitest'
import { processDecodes } from './alerts'
import { pushToast } from './toast'
import type { DecodeRow, Settings } from './types'

vi.mock('./toast', () => ({ pushToast: vi.fn() }))
const toasts = vi.mocked(pushToast)

// Node test env has no window; alerts.ts only needs setTimeout + (optional)
// AudioContext, and ensureCtx degrades to silent when the latter is absent.
vi.stubGlobal('window', { setTimeout } as unknown as Window & typeof globalThis)

const settings = { alertMyCall: true, alertNew: true, alertCq: false } as unknown as Settings

let seq = 0
function decode(over: Partial<DecodeRow>): DecodeRow {
  // Unique message per row so the exact-decode dedup never hides a test case.
  return {
    from: 'F5XYZ',
    message: `msg-${seq++}`,
    freqHz: 1500 + seq,
    directedToMe: false,
    newDxcc: false,
    newGrid: false,
    isCq: false,
    ...over,
  } as unknown as DecodeRow
}

beforeEach(() => toasts.mockClear())

describe('processDecodes QSO-aware quieting', () => {
  it('alerts "calling you" while idle/monitoring', () => {
    processDecodes([decode({ directedToMe: true })], settings, undefined, {
      state: 'Listening',
      dxcall: null,
    })
    expect(toasts).toHaveBeenCalledTimes(1)
    expect(toasts.mock.calls[0][0]).toContain('calling you')
  })

  it('suppresses "calling you" while mid-QSO or running CQ', () => {
    for (const state of ['CallingCq', 'AwaitReport', 'AwaitRoger', 'Confirming']) {
      processDecodes([decode({ from: 'K1ABC', directedToMe: true })], settings, undefined, {
        state,
        dxcall: null,
      })
    }
    expect(toasts).not.toHaveBeenCalled()
  })

  it('suppresses "calling you" during a Field Day exchange too (FD state strings)', () => {
    for (const state of ['CallingCq', 'AwaitExchange', 'AwaitConfirm']) {
      processDecodes([decode({ from: 'W1AW', directedToMe: true })], settings, undefined, {
        state,
        dxcall: 'W1AW',
      })
    }
    expect(toasts).not.toHaveBeenCalled()
  })

  it('never pops anything about the station currently being worked', () => {
    processDecodes(
      [decode({ from: 'F5XYZ', directedToMe: true, newDxcc: true })],
      settings,
      undefined,
      { state: 'AwaitRoger', dxcall: 'f5xyz' }, // case-insensitive match
    )
    expect(toasts).not.toHaveBeenCalled()
  })

  it('still fires the loud new-DXCC alert for OTHER stations while engaged', () => {
    processDecodes(
      [decode({ from: 'ZL9DX', newDxcc: true, country: 'Auckland Is.' })],
      settings,
      undefined,
      { state: 'AwaitReport', dxcall: 'F5XYZ' },
    )
    expect(toasts).toHaveBeenCalledTimes(1)
    expect(toasts.mock.calls[0][0]).toContain('NEW DXCC')
  })

  it('behaves as before when no QSO context is passed', () => {
    processDecodes([decode({ from: 'W1AW', directedToMe: true })], settings)
    expect(toasts).toHaveBeenCalledTimes(1)
  })
})
