import { expect, fake, openApp, test } from './fixtures'

test.describe('Apps', () => {
  test('runs Kokoro from the gallery on a runtime it starts itself', async ({ page }) => {
    await openApp(page, { connected: true }, '/#/apps')
    await page.getByRole('link', { name: /Kokoro Text to Speech/ }).click()
    await expect(page.getByText(/No runtime yet/)).toBeVisible()

    await page.getByRole('button', { name: 'Product intro' }).click()
    await expect(page.getByLabel(/Text/)).toHaveValue(/Meet NZAP Engine/)
    await page.getByRole('button', { name: /Generate speech/ }).click()

    await expect(page.getByText(/Runtime app-kokoro-tts is ready/)).toBeVisible()
    await expect(page.getByRole('button', { name: 'Play' })).toBeVisible()
    await expect(page.getByRole('slider', { name: 'Seek' })).toBeVisible()
    const created = await fake(page, (state) =>
      [...state.sessions.values()].map((session) => session.accelerator),
    )
    expect(created).toEqual(['T4'])

    // A second run on the same runtime is warm.
    await page.getByRole('button', { name: /Generate speech/ }).click()
    await expect(page.getByText('Already warm')).toBeVisible()
    await expect(page.getByText(/Earlier results \(1\)/)).toBeVisible()

    // The gallery shows the warm runtime.
    await page.getByRole('link', { name: 'All apps' }).click()
    await expect(
      page.getByRole('link', { name: /Kokoro Text to Speech/ }).getByText('Warm'),
    ).toBeVisible()
  })

  test('opens an app from the notebook list', async ({ page }) => {
    await openApp(page, { connected: true })
    await page.getByRole('tab', { name: 'Notebooks' }).click()
    await page
      .getByRole('listitem')
      .filter({ hasText: 'Kokoro Text to Speech' })
      .getByRole('link', { name: 'Open app' })
      .click()
    await expect(page.getByRole('heading', { name: 'Kokoro Text to Speech' })).toBeVisible()
  })
})
