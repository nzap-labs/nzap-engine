import { expect, fake, openApp, test } from './fixtures'

test.describe('Connecting Google', () => {
  test('first launch shows onboarding and connects through the browser', async ({ page }) => {
    await openApp(page)
    await expect(page.getByRole('region', { name: 'Welcome' })).toBeVisible()
    await expect(page.getByText('Google Auth off')).toBeVisible()
    // Nothing to do before connecting: the workspace tabs are hidden.
    await expect(page.getByRole('tab', { name: 'Runtimes' })).toHaveCount(0)

    await page.getByRole('button', { name: 'Connect Google' }).click()
    await expect(page.getByRole('button', { name: /Waiting for Google/ })).toBeVisible()
    await expect(page.getByText('Google Auth connected')).toBeVisible()
    expect(await fake(page, (state) => state.opened[0])).toContain('accounts.google.com')

    await expect(page.getByRole('region', { name: 'Welcome' })).toHaveCount(0)
    await expect(page.getByRole('tab', { name: 'Runtimes' })).toBeVisible()
    // The sidebar shows who is connected.
    const identity = page.getByRole('button', { name: /Ada Lovelace/ })
    await expect(identity).toBeVisible()
    await expect(identity).toContainText('Google connected')
  })

  test('disconnecting releases runtimes and returns to onboarding', async ({ page }) => {
    await openApp(page, { connected: true })
    await page.getByPlaceholder('my-runtime').fill('box')
    await page.getByRole('button', { name: 'Launch runtime' }).click()
    await expect(page.getByText('Runtime box is ready')).toBeVisible()

    await page.getByRole('button', { name: 'Disconnect' }).click()
    const dialog = page.getByRole('dialog', { name: 'Disconnect Google Auth?' })
    await dialog.getByRole('button', { name: 'Disconnect' }).click()
    await expect(page.getByText('Google Auth disconnected.')).toBeVisible()
    await expect(page.getByRole('region', { name: 'Welcome' })).toBeVisible()
    expect(await fake(page, (state) => state.sessions.size)).toBe(0)
  })

  test('the copy/paste fallback connects too', async ({ page }) => {
    await openApp(page)
    await page.getByRole('button', { name: 'Use a code' }).click()
    const dialog = page.getByRole('dialog', { name: 'Connect with a code' })
    await dialog.getByRole('button', { name: /Open Google sign-in/ }).click()
    await dialog.getByLabel('Authorization code').fill('4/0AbCdE')
    await dialog.getByRole('button', { name: 'Connect' }).click()
    await expect(page.getByText('Google Auth connected')).toBeVisible()
  })
})
