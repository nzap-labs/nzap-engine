import { screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { renderWithEngine } from '@/test/render'
import { AppPage } from './app-page'
import { AppsPage } from './apps-page'
import { resetAppStore } from './store'

beforeEach(() => {
  resetAppStore()
  // jsdom has no object URLs; the player only needs a string to point at.
  URL.createObjectURL = vi.fn(() => 'blob:nzap-test')
  URL.revokeObjectURL = vi.fn()
})

describe('Apps', () => {
  it('lists apps from the collection with their runtime and timing', async () => {
    renderWithEngine(<AppsPage />, { connected: true })
    const card = (await screen.findByText('Kokoro Text to Speech')).closest('a')!
    expect(within(card).getByText(/T4/)).toBeInTheDocument()
    expect(within(card).getByText(/setup ~50s/)).toBeInTheDocument()
    // Plain notebooks are not apps.
    expect(screen.queryByText('Print Notebook')).not.toBeInTheDocument()
  })

  it('starts the recommended runtime, runs the app, then runs warm', async () => {
    const { user, engine } = renderWithEngine(<AppPage appId="public:kokoro-tts" />, {
      connected: true,
    })
    expect(await screen.findByText(/No runtime yet/)).toBeInTheDocument()

    await user.click(screen.getByRole('radio', { name: 'English (UK)' }))
    expect(screen.getByRole('combobox')).toHaveValue('bf_emma')

    await user.click(screen.getByRole('button', { name: /Generate speech/ }))
    expect(await screen.findByRole('button', { name: 'Play' }, { timeout: 5000 })).toBeVisible()
    const session = [...engine.state.sessions.values()][0]
    expect(session.name).toBe('app-kokoro-tts')
    expect(session.accelerator).toBe('T4')
    expect([...session.files.keys()].some((path) => path.endsWith('.wav'))).toBe(true)
    expect(engine.state.calls).toContain('files_read')

    // The model stays loaded: the second run skips setup.
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /Generate speech/ })).toBeEnabled(),
    )
    await user.click(screen.getByRole('button', { name: /Generate speech/ }))
    expect(await screen.findByText('Already warm', {}, { timeout: 5000 })).toBeInTheDocument()
    await waitFor(() => expect(screen.getByText(/Earlier results \(1\)/)).toBeInTheDocument(), {
      timeout: 5000,
    })
  })

  it('renders table outputs', async () => {
    const { user } = renderWithEngine(<AppPage appId="public:hf-sentiment" />, {
      connected: true,
    })
    await user.click(await screen.findByRole('button', { name: /Score sentiment/ }))
    const table = await screen.findByRole('table', {}, { timeout: 5000 })
    expect(within(table).getByText('Confidence')).toBeInTheDocument()
    expect(within(table).getAllByText('Positive').length).toBeGreaterThan(0)
  })

  it('asks to connect Google first', async () => {
    renderWithEngine(<AppPage appId="public:kokoro-tts" />)
    expect(await screen.findByText('Connect Google to run apps')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /Generate speech/ })).toBeDisabled()
  })
})
