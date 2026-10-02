import { screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it } from 'vitest'
import { renderWithEngine } from '@/test/render'
import { UpdatesCard } from './updates-card'
import { resetUpdates } from './updates'

beforeEach(() => resetUpdates())

describe('UpdatesCard', () => {
  it('reports when the app is current', async () => {
    const { user } = renderWithEngine(<UpdatesCard />)
    await user.click(await screen.findByRole('button', { name: /Check for updates/ }))
    expect(await screen.findByText('You have the latest version.')).toBeInTheDocument()
  })

  it('installs an available update and relaunches', async () => {
    const { user, engine } = renderWithEngine(<UpdatesCard />, {
      update: { version: '0.2.0', body: 'Apps and a new look.', date: '2026-10-10T00:00:00Z' },
    })
    await user.click(await screen.findByRole('button', { name: /Check for updates/ }))
    expect(await screen.findByText('Version 0.2.0 is available')).toBeInTheDocument()
    expect(screen.getByText('Apps and a new look.')).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: /Install and restart/ }))
    await waitFor(() => expect(engine.state.restarted).toBe(true))
    expect(engine.state.calls).toContain('plugin:updater|download_and_install')
    expect(screen.getByText(/Restarting into the new version/)).toBeInTheDocument()
  })
})
