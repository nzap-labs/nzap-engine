import { screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { renderWithEngine } from '@/test/render'
import { NewRuntimeCard } from './new-runtime-card'

describe('NewRuntimeCard', () => {
  it('needs a connected Google account', async () => {
    renderWithEngine(<NewRuntimeCard />)
    expect(await screen.findByText('Connect Google Auth first.')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /Launch runtime/ })).toBeDisabled()
  })

  it('launches a GPU runtime and disables accelerators the plan lacks', async () => {
    const onCreated = vi.fn()
    const { user, engine } = renderWithEngine(<NewRuntimeCard onCreated={onCreated} />, {
      connected: true,
    })
    await user.type(await screen.findByPlaceholderText('my-runtime'), 'trainer')
    await user.click(screen.getByRole('radio', { name: 'gpu' }))

    const a100 = await screen.findByRole('option', { name: /A100 — not available on your plan/ })
    expect(a100).toBeDisabled()
    expect(screen.getByRole('combobox')).toHaveValue('t4')

    await user.click(screen.getByRole('checkbox', { name: 'High-RAM shape' }))
    await user.click(screen.getByRole('button', { name: /Launch runtime/ }))
    await waitFor(() => expect(onCreated).toHaveBeenCalledWith('trainer'))
    const created = engine.state.sessions.get('trainer')!
    expect(created.accelerator).toBe('T4')
    expect(created.shape).toBe('High-RAM')
  })

  it('shows why a launch failed', async () => {
    const { user } = renderWithEngine(<NewRuntimeCard />, {
      connected: true,
      failNextCreate: {
        code: 'too_many_runtimes',
        message: 'This account already holds the maximum number of Colab VMs.',
      },
    })
    await user.click(await screen.findByRole('button', { name: /Launch runtime/ }))
    expect(await screen.findByText(/maximum number of Colab VMs/)).toBeInTheDocument()
  })
})
