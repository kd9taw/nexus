// @vitest-environment jsdom
import { describe, it, expect, vi, beforeAll } from 'vitest'
import { render, waitFor } from '@testing-library/react'
import { Logbook } from './Logbook'
import * as api from '../api'

// react-virtual (virtual-core) measures the scroll element and rows via offsetHeight + a
// ResizeObserver, neither of which jsdom implements — stub them so a non-trivial visible window is
// computed. offsetHeight (not getBoundingClientRect) is what virtual-core actually reads.
beforeAll(() => {
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver
  Object.defineProperty(HTMLElement.prototype, 'offsetHeight', { configurable: true, value: 600 })
  Object.defineProperty(HTMLElement.prototype, 'offsetWidth', { configurable: true, value: 900 })
})

vi.mock('../api', () => {
  const noop = () => vi.fn()
  return {
    getLog: vi.fn(),
    deleteQso: noop(), editQso: noop(), exportGeneralLog: noop(), importAdif: noop(),
    logQso: noop(), markQslSent: noop(), purgeLog: noop(), qrzLookup: noop(),
    syncLotwReport: noop(), uploadLotwReport: noop(), qrzPushQso: noop(),
    clublogPushQso: noop(), hrdlogPushQso: noop(),
  }
})
vi.mock('../toast', () => ({ pushToast: vi.fn(), withErrorToast: vi.fn() }))

function fakeLog(n: number) {
  return Array.from({ length: n }, (_, i) => ({
    call: `K${i}ABC`,
    grid: 'EN37',
    band: '20m',
    freqMhz: 14.074 + i * 1e-6,
    mode: 'FT8',
    rstSent: '-10',
    rstRcvd: '-12',
    name: null,
    qth: null,
    comment: null,
    notes: null,
    country: 'United States',
    whenUnix: 1_700_000_000 + i,
    confirmed: false,
    awardConfirmed: false,
    qslRcvd: null,
    qslSent: null,
    ota: null,
    upload: undefined,
  }))
}

describe('Logbook virtualization', () => {
  it('mounts only a small window of rows for a large (5k) log', async () => {
    const N = 5000
    ;(api.getLog as ReturnType<typeof vi.fn>).mockResolvedValue(fakeLog(N))
    const { container } = render(
      <Logbook defaultBand="20m" defaultFreqMhz={14.074} defaultMode="FT8" />,
    )
    // Wait for the async getLog → setLog → virtualized render (the spacer div appears only once the
    // log has loaded; before that .log-scroll's child is the "no contacts" <p>).
    await waitFor(() => expect(container.querySelector('.log-scroll > div')).not.toBeNull())
    // The virtualizer is engaged over the FULL set — the spacer reserves the whole scroll height
    // (~5000 rows) even though only a small window is realized.
    const spacer = container.querySelector('.log-rows') as HTMLElement
    expect(parseInt(spacer.style.height, 10)).toBeGreaterThan(5000 * 30)
    // A real (small) window is realized — proves rows actually render (catches a dropped scroll
    // ref / broken wiring, which would leave getVirtualItems() empty while the spacer still sized).
    const mounted = container.querySelectorAll('.logbook-row:not(.head)').length
    expect(mounted).toBeGreaterThan(0)
    // ...but nowhere near all 5000 — the whole point of virtualization.
    expect(mounted).toBeLessThan(200)
    // Default sort is newest-first, so the top of the window shows the highest-whenUnix call.
    expect(container.textContent).toContain('K4999ABC')
  })
})
