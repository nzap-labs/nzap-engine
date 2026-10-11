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

  it('needs the provider connected, not Google, for Codespaces', async () => {
    const { user } = renderWithEngine(<NewRuntimeCard />)
    await user.click(await screen.findByRole('radio', { name: 'Codespaces' }))
    expect(
      await screen.findByText('Connect GitHub Codespaces in Settings → Compute providers first.'),
    ).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /Launch runtime/ })).toBeDisabled()
    // No Google connection is needed for other providers.
    expect(screen.queryByText('Connect Google Auth first.')).not.toBeInTheDocument()
    expect(screen.queryByRole('radiogroup', { name: 'Hardware' })).not.toBeInTheDocument()
  })

  it('creates a codespace with the chosen machine', async () => {
    const onCreated = vi.fn()
    const { user, engine } = renderWithEngine(<NewRuntimeCard onCreated={onCreated} />, {
      github: { login: 'ada' },
    })
    await user.click(await screen.findByRole('radio', { name: 'Codespaces' }))
    await user.type(screen.getByPlaceholderText('my-runtime'), 'cs-box')
    const machine = screen.getByRole('combobox', { name: 'Machine' })
    await screen.findByRole('option', { name: /4 cores/ })
    await user.selectOptions(machine, 'standardLinux32gb')
    await user.click(screen.getByRole('button', { name: /Launch runtime/ }))
    await waitFor(() => expect(onCreated).toHaveBeenCalledWith('cs-box'))
    const created = engine.state.sessions.get('cs-box')!
    expect(created.provider).toBe('codespaces')
    expect(created.machine).toBe('standardLinux32gb')
  })

  it('creates a CodeSandbox VM with the default tier', async () => {
    const { user, engine } = renderWithEngine(<NewRuntimeCard />, { codesandbox: true })
    await user.click(await screen.findByRole('radio', { name: 'CodeSandbox' }))
    await user.type(screen.getByPlaceholderText('my-runtime'), 'sandbox')
    await user.click(screen.getByRole('button', { name: /Launch runtime/ }))
    await waitFor(() => expect(engine.state.sessions.get('sandbox')?.machine).toBe('Nano'))
    expect(engine.state.sessions.get('sandbox')?.provider).toBe('codesandbox')
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
