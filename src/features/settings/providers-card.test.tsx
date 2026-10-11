import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { renderWithEngine } from '@/test/render'
import { SettingsPage } from './settings-page'

describe('Compute providers', () => {
  it('connects GitHub with a token or the GitHub CLI, and disconnects', async () => {
    const { user, engine } = renderWithEngine(<SettingsPage />)
    const github = await screen.findByRole('group', { name: 'GitHub Codespaces' })
    expect(within(github).getByText('Not connected')).toBeVisible()

    await user.type(within(github).getByLabelText('GitHub token'), 'not-a-token')
    await user.click(within(github).getByRole('button', { name: /Use token/ }))
    expect(await screen.findByText(/GitHub rejected the stored token/)).toBeInTheDocument()

    await user.clear(within(github).getByLabelText('GitHub token'))
    await user.type(within(github).getByLabelText('GitHub token'), 'ghp_valid')
    await user.click(within(github).getByRole('button', { name: /Use token/ }))
    expect(await screen.findByText('GitHub connected as ada.')).toBeInTheDocument()
    expect(await within(github).findByText('Connected as ada')).toBeVisible()
    expect(engine.state.github).toEqual({ login: 'ada' })

    await user.click(within(github).getByRole('button', { name: /Disconnect GitHub/ }))
    expect(await within(github).findByText('Not connected')).toBeVisible()
    await user.click(within(github).getByRole('button', { name: /Use GitHub CLI login/ }))
    expect(await within(github).findByText('Connected as ada')).toBeVisible()
  })

  it('signs in with the device flow once an OAuth app is set', async () => {
    // Slow enough to read the code before the simulated approval.
    const { user, engine } = renderWithEngine(<SettingsPage />, { delay: 150 })
    const github = await screen.findByRole('group', { name: 'GitHub Codespaces' })
    await user.click(within(github).getByRole('button', { name: /Sign in with GitHub/ }))
    expect(
      await screen.findByText(/needs a GitHub OAuth app/, {}, { timeout: 3000 }),
    ).toBeInTheDocument()

    await user.type(within(github).getByLabelText('GitHub OAuth client id'), 'Iv1.nzap')
    await user.click(within(github).getAllByRole('button', { name: /Save/ })[0])
    await waitFor(() => expect(engine.state.settings.githubClientId).toBe('Iv1.nzap'))

    const refreshed = await screen.findByRole('group', { name: 'GitHub Codespaces' })
    await user.click(within(refreshed).getByRole('button', { name: /Sign in with GitHub/ }))
    expect(await screen.findByText('WDJB-MJHT', {}, { timeout: 3000 })).toBeVisible()
    expect(engine.state.opened).toContain('https://github.com/login/device')
    await waitFor(() => expect(engine.state.github).toEqual({ login: 'ada' }), { timeout: 3000 })
  })

  it('stores a CodeSandbox key, provider defaults and the auto-delete delay', async () => {
    const { user, engine } = renderWithEngine(<SettingsPage />)
    const csb = await screen.findByRole('group', { name: 'CodeSandbox' })
    await user.type(within(csb).getByLabelText('CodeSandbox API key'), 'csb_key')
    await user.click(within(csb).getByRole('button', { name: /Save key/ }))
    expect(await within(csb).findByText('Connected')).toBeVisible()
    expect(engine.state.codesandbox).toBe(true)

    await user.selectOptions(within(csb).getByLabelText('CodeSandbox VM tier'), 'Micro')
    await user.click(within(csb).getByRole('button', { name: /^Save$/ }))
    await waitFor(() => expect(engine.state.settings.codesandboxTier).toBe('Micro'))

    const card = screen.getByRole('region', { name: 'Compute providers' })
    const minutes = within(card).getByRole('spinbutton')
    await user.clear(minutes)
    await user.type(minutes, '0')
    await user.click(
      within(card)
        .getAllByRole('button', { name: /^Save$/ })
        .at(-1)!,
    )
    expect(
      await screen.findByText('Idle runtimes are no longer deleted automatically.'),
    ).toBeInTheDocument()
    expect(engine.state.settings.autoReleaseMinutes).toBe(0)
  })
})
