import { mkdirSync } from 'node:fs'
import path from 'node:path'
import { expect, openApp, test } from '../web/fixtures'

/**
 * Product screenshots for the README, the website and the launch video.
 * Not part of CI: `npx playwright test -c e2e/capture/playwright.config.ts`.
 * Writes PNGs to `CAPTURE_DIR` (default: test-results/capture).
 */
const OUT = process.env.CAPTURE_DIR ?? path.resolve('test-results/capture')
mkdirSync(OUT, { recursive: true })

for (const theme of ['dark', 'light'] as const) {
  test(`connect screen (${theme})`, async ({ page }) => {
    await page.addInitScript((value) => localStorage.setItem('nzap.theme', value), theme)
    await openApp(page, { connected: false }, '/#/colab')
    await page.waitForTimeout(400)
    await page.screenshot({ path: path.join(OUT, `${theme}-01-connect.png`) })
  })

  test(`product screens (${theme})`, async ({ page }) => {
    await page.addInitScript((value) => localStorage.setItem('nzap.theme', value), theme)
    const shot = async (name: string) => {
      await page.evaluate(() =>
        document.querySelectorAll('.overflow-y-auto').forEach((element) => element.scrollTo(0, 0)),
      )
      await page.screenshot({ path: path.join(OUT, `${theme}-${name}.png`) })
    }

    await openApp(page, { connected: true, delay: 60 }, '/#/apps')
    await expect(page.getByRole('link', { name: /Kokoro Text to Speech/ })).toBeVisible()
    await page.waitForTimeout(300)
    await shot('02-apps')

    await page.getByRole('link', { name: /Kokoro Text to Speech/ }).click()
    await page.getByRole('button', { name: 'Product intro' }).click()
    await page.waitForTimeout(300)
    await shot('03-app-form')

    await page.getByRole('button', { name: /Generate speech/ }).click()
    await expect(page.getByText(/Installing|Downloading|Loading/).first()).toBeVisible()
    await page.waitForTimeout(700)
    await shot('04-app-running')

    await expect(page.getByRole('button', { name: 'Play' })).toBeVisible({ timeout: 20_000 })
    await page.waitForTimeout(600)
    await shot('05-app-result')

    await page.getByRole('button', { name: /Generate speech/ }).click()
    await expect(page.getByText('Already warm')).toBeVisible()
    await page.waitForTimeout(250)
    await shot('06-app-warm')

    await page.goto('/#/colab')
    await page.getByRole('tab', { name: 'Runtimes' }).click()
    await page.waitForTimeout(400)
    await shot('07-runtimes')

    await page.getByRole('tab', { name: 'Notebooks' }).click()
    await page.waitForTimeout(400)
    await shot('08-notebooks')
  })
}
