import { describe, it, expect, vi, beforeEach } from 'vitest'
import { processFlare, effectiveXray, resetFlareAlerts } from './flareAlert'
import { pushToast } from './toast'
import { doubleBeep } from './alerts'

vi.mock('./toast', () => ({ pushToast: vi.fn() }))
vi.mock('./alerts', () => ({ doubleBeep: vi.fn() }))

const toasts = vi.mocked(pushToast)
const beeps = vi.mocked(doubleBeep)

beforeEach(() => {
  resetFlareAlerts()
  toasts.mockClear()
  beeps.mockClear()
})

describe('processFlare (edge-triggered flare heads-up)', () => {
  it('quiet toast, no beep, for an M1–M4 (tier 1) onset — once per event', () => {
    processFlare(2e-5) // M2
    expect(toasts).toHaveBeenCalledTimes(1)
    expect(toasts.mock.calls[0][0]).toContain('M2.0')
    expect(toasts.mock.calls[0][0]).toContain('R1')
    expect(toasts.mock.calls[0][1]).toBe('info')
    expect(beeps).not.toHaveBeenCalled()
    processFlare(3e-5) // still tier 1 → silent
    processFlare(1.2e-5)
    expect(toasts).toHaveBeenCalledTimes(1)
  })

  it('prominent toast + beep at M5+ and again on escalation to X', () => {
    processFlare(6e-5) // M6 (tier 2)
    expect(beeps).toHaveBeenCalledTimes(1)
    expect(toasts).toHaveBeenCalledTimes(1)
    expect(toasts.mock.calls[0][3]).toMatchObject({ prominent: true })
    processFlare(2e-4) // X2 (tier 3) — the escalation re-alert
    expect(toasts).toHaveBeenCalledTimes(2)
    expect(toasts.mock.calls[1][0]).toContain('X2.0')
    expect(toasts.mock.calls[1][0]).toContain('R3')
    processFlare(1.5e-4) // still tier 3 → silent
    expect(toasts).toHaveBeenCalledTimes(2)
  })

  it('does not re-fire during decay; re-arms only after flux holds below C5', () => {
    processFlare(2e-5) // M2 → toast
    processFlare(8e-6) // decayed below M1 but above C5 → not yet re-armed
    processFlare(2e-5) // wobble back up → same event, no second toast
    expect(toasts).toHaveBeenCalledTimes(1)
    processFlare(1e-6) // below C5 → event over, re-armed
    vi.spyOn(Date, 'now').mockReturnValue(Date.now() + 61 * 60_000) // past cooldown
    processFlare(2e-5) // a NEW event → alerts again
    expect(toasts).toHaveBeenCalledTimes(2)
    vi.restoreAllMocks()
  })

  it('a cooldown-suppressed second event stays ARMED and fires once the cooldown lapses', () => {
    // The clustered-flare case (one active region, multiple M/X within an hour):
    // suppression must delay the alert, never permanently drop it.
    const t0 = Date.now()
    const clock = vi.spyOn(Date, 'now')
    clock.mockReturnValue(t0)
    processFlare(6e-5) // M6 fires (tier 2)
    expect(toasts).toHaveBeenCalledTimes(1)
    processFlare(1e-6) // below C5 → event over, re-armed
    clock.mockReturnValue(t0 + 35 * 60_000)
    processFlare(6e-5) // second M6, inside the 60-min cooldown → suppressed for now
    expect(toasts).toHaveBeenCalledTimes(1)
    clock.mockReturnValue(t0 + 61 * 60_000)
    processFlare(6e-5) // still M6, cooldown lapsed → the delayed alert fires
    expect(toasts).toHaveBeenCalledTimes(2)
    processFlare(6e-5) // and only once
    expect(toasts).toHaveBeenCalledTimes(2)
    clock.mockRestore()
  })

  it('includes the D-RAP ceiling and recovery estimate in the copy', () => {
    processFlare(1e-4) // X1 → HAF 25 MHz, recovery ≈58 min
    const msg = toasts.mock.calls[0][0]
    expect(msg).toContain('below ~25 MHz')
    expect(msg).toMatch(/fade ~5[0-9] min/)
  })

  it('ignores null / quiet-sun readings', () => {
    processFlare(null)
    processFlare(1e-7)
    expect(toasts).not.toHaveBeenCalled()
  })
})

describe('effectiveXray (fast lane / snapshot merge)', () => {
  it('prefers the fresher fast-lane reading, falls back to the snapshot', () => {
    expect(effectiveXray(2e-5, 1e-4)).toBe(2e-5)
    expect(effectiveXray(null, 1e-4)).toBe(1e-4)
    expect(effectiveXray(undefined, undefined)).toBeNull()
    expect(effectiveXray(0, 0)).toBeNull()
  })
})
