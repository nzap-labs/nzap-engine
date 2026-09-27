import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { renderWithEngine } from '@/test/render'
import { ConnectionCard } from './connection-card'

describe('ConnectionCard', () => {
  it('connects Google through the browser round-trip', async () => {
    const { user, engine } = renderWithEngine(<ConnectionCard />)
    const card = await screen.findByRole('region', { name: 'Google Auth' })
    await within(card).findByText('not connected')

    await user.click(screen.getByRole('button', { name: /Connect Google/ }))
    // The consent page was opened in the browser, not inside the app.
    expect(engine.state.opened[0]).toContain('accounts.google.com')

    await within(card).findByText('connected')
    expect(card).toHaveTextContent('Connected as ada@example.com')
    expect(card).toHaveTextContent('in your system keychain')
    // Plan details come from the normalised quota.
    expect(await within(card).findByText('36.0 free')).toBeInTheDocument()
  })

  it('disconnects after confirmation', async () => {
    const { user, engine } = renderWithEngine(<ConnectionCard />, { connected: true })
    await user.click(await screen.findByRole('button', { name: /Disconnect/ }))
    const dialog = screen.getByRole('dialog', { name: 'Disconnect Google Auth?' })
    expect(dialog).toHaveTextContent('releases every Colab runtime')
    await user.click(within(dialog).getByRole('button', { name: 'Disconnect' }))
    await waitFor(() => expect(engine.state.connected).toBe(false))
    expect(await screen.findByText('not connected')).toBeInTheDocument()
  })

  it('offers the copy/paste flow', async () => {
    const { user, engine } = renderWithEngine(<ConnectionCard />)
    await user.click(await screen.findByRole('button', { name: /Use a code/ }))
    const dialog = screen.getByRole('dialog', { name: 'Connect with a code' })
    const submit = within(dialog).getByRole('button', { name: 'Connect' })
    expect(submit).toBeDisabled()

    await user.click(within(dialog).getByRole('button', { name: /Open Google sign-in/ }))
    expect(engine.state.opened.at(-1)).toContain('token_usage=remote')
    await user.type(within(dialog).getByLabelText('Authorization code'), '4/0Abc')
    await user.click(submit)
    await waitFor(() => expect(engine.state.connected).toBe(true))
  })
})
