import { screen, waitFor } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { call } from '@/lib/ipc'
import { renderWithEngine } from '@/test/render'
import { ConsolePanel } from './console-panel'

async function withRuntime(state: Parameters<typeof renderWithEngine>[1] = {}) {
  const rendered = renderWithEngine(<ConsolePanel sessionName="box" />, {
    connected: true,
    ...state,
  })
  await call('session_create', { request: { name: 'box' } })
  return rendered
}

async function runCell(user: ReturnType<typeof renderWithEngine>['user'], code: string) {
  const editor = screen.getByLabelText('Code')
  await user.clear(editor)
  await user.type(
    editor,
    code.replace(/[{[]/g, (char) => char + char),
  )
  await user.click(screen.getByRole('button', { name: /Run cell/ }))
}

describe('ConsolePanel', () => {
  it('asks for a runtime first', () => {
    renderWithEngine(<ConsolePanel sessionName={null} />)
    expect(screen.getByText('Select a runtime to run code.')).toBeInTheDocument()
  })

  it('streams output and marks the cell finished', async () => {
    const { user } = await withRuntime()
    await runCell(user, 'print("hi")\nanswer')
    expect(await screen.findByText(/finished/)).toHaveTextContent('✓ finished')
    expect(screen.getByText('hi')).toBeInTheDocument()
    expect(screen.getByText('42')).toBeInTheDocument()
    expect(screen.getByText('Out [1]')).toBeInTheDocument()
  })

  it('shows tracebacks for failing cells', async () => {
    const { user } = await withRuntime()
    await runCell(user, 'fail()')
    expect(await screen.findByText('✗ finished with errors')).toBeInTheDocument()
    expect(screen.getByText(/ValueError: boom/)).toBeInTheDocument()
  })

  it('answers input() prompts', async () => {
    const { user } = await withRuntime()
    await runCell(user, 'name = input("Name? ")')
    const answer = await screen.findByPlaceholderText('type your answer and press Enter')
    expect(screen.getByText('Name?')).toBeInTheDocument()
    await user.type(answer, 'Ada{Enter}')
    expect(await screen.findByText('Hello, Ada!')).toBeInTheDocument()
  })

  it('pauses on Drive consent and resumes after Continue', async () => {
    const { user, engine } = await withRuntime({ driveConsent: false })
    await runCell(user, "drive.mount('/content/drive')")
    const approve = await screen.findByRole('link', { name: 'Approve access' })
    expect(approve).toHaveAttribute('href', 'https://accounts.google.com/o/oauth2/consent?fake=1')

    // Continue before approving: still refused.
    await user.click(screen.getByRole('button', { name: 'Continue' }))
    await waitFor(() => expect(engine.state.calls).toContain('session_drive_authorize'))
    expect(screen.queryByText('Mounted at /content/drive')).not.toBeInTheDocument()

    engine.state.driveConsent = true
    await user.click(screen.getByRole('button', { name: 'Continue' }))
    expect(await screen.findByText('Mounted at /content/drive')).toBeInTheDocument()
    expect(screen.queryByRole('link', { name: 'Approve access' })).not.toBeInTheDocument()
  })

  it('interrupts a running cell', async () => {
    const { user, engine } = await withRuntime()
    await runCell(user, 'wait_for_interrupt()')
    const interrupt = screen.getByRole('button', { name: /Interrupt/ })
    await waitFor(() => expect(interrupt).toBeEnabled())
    await user.click(interrupt)
    await waitFor(() => expect(engine.state.calls).toContain('session_interrupt'))
    expect(await screen.findByRole('button', { name: /Run cell/ })).toBeEnabled()
  })
})
