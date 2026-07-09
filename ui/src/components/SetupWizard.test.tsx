// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { SetupWizard } from './SetupWizard'
import * as api from '../api'

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
