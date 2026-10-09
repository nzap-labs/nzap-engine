import { screen, waitFor } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { renderWithEngine } from '@/test/render'
import { SettingsPage } from './settings-page'

describe('SettingsPage', () => {
  it('turns keep-alive and close-to-tray on and off', async () => {
    const { user, engine } = renderWithEngine(<SettingsPage />)
    const keepAlive = await screen.findByRole('checkbox', {
      name: 'Keep runtimes alive while NZAP Engine is open',
    })
    const tray = screen.getByRole('checkbox', {
      name: 'Keep running in the system tray when the window is closed',
    })
    expect(tray).not.toBeChecked()

    await user.click(tray)
    expect(
      await screen.findByText('Closing the window now keeps NZAP Engine in the system tray.'),
    ).toBeInTheDocument()
    expect(engine.state.settings.closeToTray).toBe(true)
    await waitFor(() =>
      expect(
        screen.getByRole('checkbox', {
          name: 'Keep running in the system tray when the window is closed',
        }),
      ).toBeChecked(),
    )

    const wasOn = engine.state.settings.keepAlive
    await user.click(keepAlive)
    await waitFor(() => expect(engine.state.settings.keepAlive).toBe(!wasOn))
  })

  it('shows how to connect an AI agent and copies it', async () => {
    const { user } = renderWithEngine(<SettingsPage />)
    expect(await screen.findByRole('region', { name: 'AI agents (MCP)' })).toBeVisible()
    const command =
      "claude mcp add --scope user nzap -- '/Applications/NZAP Engine.app/Contents/MacOS/nzap-engine' mcp"
    expect(await screen.findByText(command)).toBeVisible()
    expect(screen.getByText(/"mcpServers"/)).toBeVisible()

    await user.click(screen.getByRole('button', { name: 'Copy Claude Code' }))
    // user-event provides the clipboard.
    expect(await navigator.clipboard.readText()).toBe(command)
    expect(await screen.findByText('Claude Code configuration copied.')).toBeInTheDocument()
  })

  it('rejects a catalog URL that is not https', async () => {
    const { user, engine } = renderWithEngine(<SettingsPage />)
    const field = await screen.findByRole('textbox', { name: 'Catalog URL' })
    const before = engine.state.settings.catalogUrl
    await user.clear(field)
    await user.type(field, 'http://example.com/catalog/')
    await user.click(screen.getAllByRole('button', { name: /Save/ })[1])
    expect(await screen.findByText('The catalog URL must be an https:// address.')).toBeVisible()
    expect(engine.state.settings.catalogUrl).toBe(before)
  })
})
