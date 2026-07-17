// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent, waitFor, cleanup } from '@testing-library/react'
import { SetupWizard } from './SetupWizard'
import * as api from '../api'
import { memoriesStore, emptyBank, addMemory } from '../features/memories'

vi.mock('../api', () => ({
  importAdif: vi.fn(),
  detectRigs: vi.fn(() => Promise.resolve([])),
  discoverFlex: vi.fn(() => Promise.resolve([])),
  getAudioDevices: vi.fn(() => Promise.resolve({ input: [], output: [] })),
}))

const importAdif = api.importAdif as ReturnType<typeof vi.fn>

function renderWizard() {
  const onApply = vi.fn()
  const onSkip = vi.fn()
  render(
    <SetupWizard
      settings={null}
      onApply={onApply}
      onTestCat={vi.fn(() => Promise.resolve({} as never))}
      onSkip={onSkip}
    />,
  )
  return { onApply, onSkip }
}

const clickNext = () => fireEvent.click(screen.getByRole('button', { name: /Next/ }))
function gotoLogStep() {
  clickNext() // 0 station → 1 rig
  clickNext() // 1 rig → 2 log
}
function fireImport(content: string) {
  const input = document.querySelector('input[type="file"]') as HTMLInputElement
  fireEvent.change(input, {
    target: { files: [new File([content], 'log.adi', { type: 'text/plain' })] },
  })
}

describe('SetupWizard ADIF import step', () => {
  beforeEach(() => importAdif.mockReset())

  it('renders the optional log step and imports an ADIF file, reporting the count', async () => {
    importAdif.mockResolvedValue({ added: 5, skipped: 1, total: 6 })
    renderWizard()
    gotoLogStep()
    expect(screen.getByText(/Bring in your existing log/)).toBeTruthy()
    fireImport('<call:5>K1ABC<eor>')
    await waitFor(() => expect(importAdif).toHaveBeenCalledTimes(1))
    const result = await screen.findByText(/Imported/)
    expect(result.textContent).toContain('5')
    expect(result.textContent).toMatch(/seeded/)
  })

  it('treats a 0-QSO import as a warning, not a false "seeded" success', async () => {
    importAdif.mockResolvedValue({ added: 0, skipped: 0, total: 0 })
    renderWizard()
    gotoLogStep()
    fireImport('this is not an ADIF file')
    expect(await screen.findByText(/No QSOs found/)).toBeTruthy()
    expect(screen.queryByText(/now seeded/)).toBeNull()
  })

  it('is skippable — Next advances to goals without importing', () => {
    renderWizard()
    gotoLogStep()
    clickNext() // 2 log → 3 goals, no file chosen
    expect(screen.getByText(/What do you mostly want to do/)).toBeTruthy()
    expect(importAdif).not.toHaveBeenCalled()
  })
})

describe('SetupWizard starter-pack offer', () => {
  const gotoGoals = () => {
    clickNext() // 0 → 1
    clickNext() // 1 → 2
    clickNext() // 2 → 3 goals
  }

  // Unmount any prior render (this file relies on auto-cleanup that isn't registered) so
  // the step-3 headings — which recur in every render — resolve to a single element.
  beforeEach(() => {
    cleanup()
    memoriesStore.set(emptyBank()) // first-run: a blank bank
  })

  it('offers packs on first run and seeds the pre-checked ones on completion', () => {
    const { onApply } = renderWizard()
    gotoGoals()
    expect(screen.getByText(/Start with some channels/)).toBeTruthy()
    // "Turn everything on (expert)" completes setup (no goal selection needed) — it must
    // seed the packs checked by default (VHF/UHF Calling + HF Digital).
    fireEvent.click(screen.getByRole('button', { name: /Turn everything on/ }))
    expect(onApply).toHaveBeenCalledTimes(1)
    const mems = memoriesStore.get().memories
    expect(mems.some((m) => m.rxMhz === 146.52)).toBe(true) // na-calling: 2m FM Calling
    expect(mems.some((m) => m.rxMhz === 14.074 && m.mode === 'FT8')).toBe(true) // na-digital
    // POTA wasn't pre-checked, so its SSB-only channels shouldn't appear.
    expect(mems.some((m) => m.rxMhz === 14.285)).toBe(false)
  })

  it('seeds nothing when the operator skips setup', () => {
    const { onSkip } = renderWizard()
    gotoGoals()
    fireEvent.click(screen.getByRole('button', { name: /set it up myself/ }))
    expect(onSkip).toHaveBeenCalledTimes(1)
    expect(memoriesStore.get().memories).toHaveLength(0)
  })

  it('hides the offer once the operator already has memories (re-open never re-adds)', () => {
    memoriesStore.set(addMemory(emptyBank(), { rxMhz: 146.52, mode: 'FM' }))
    renderWizard()
    clickNext()
    clickNext()
    clickNext()
    expect(screen.getByText(/What do you mostly want to do/)).toBeTruthy() // on the goals step
    expect(screen.queryByText(/Start with some channels/)).toBeNull()
  })
})
