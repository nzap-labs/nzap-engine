import { mkdirSync } from 'node:fs'
import path from 'node:path'
import { expect, openApp, test } from '../web/fixtures'

/**
 * Product screenshots for the README, the website and the launch video.
 * Not part of CI: `npx playwright test -c e2e/capture/playwright.config.ts`.
 * Writes PNGs to `CAPTURE_DIR` (default: e2e/capture/out, not committed).
 */
const OUT = process.env.CAPTURE_DIR ?? path.resolve('e2e/capture/out')
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

test('more screens (dark)', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('nzap.theme', 'dark'))
  const shot = async (name: string) => {
    await page.evaluate(() =>
      document.querySelectorAll('.overflow-y-auto').forEach((element) => element.scrollTo(0, 0)),
    )
    await page.screenshot({ path: path.join(OUT, `dark-${name}.png`) })
  }
  await openApp(page, { connected: true, delay: 40 }, '/#/apps')

  // A Breeze run first, so its runtime and outputs exist for the Files tab.
  await page.getByRole('link', { name: /Breeze TTS 2/ }).click()
  await page.getByRole('button', { name: 'Calm narrator' }).click()
  await page.waitForTimeout(300)
  await shot('12-breeze-form')
  await page.getByRole('button', { name: /Generate speech/ }).click()
  await expect(page.getByRole('button', { name: 'Play' })).toBeVisible({ timeout: 20_000 })
  await page.waitForTimeout(600)
  await shot('13-breeze-result')

  await page.goto('/#/colab')
  await page.getByRole('region', { name: 'Runtimes' }).getByText('app-breeze-tts').first().click()
  await page.getByLabel('Code').fill('import torch\nprint(torch.cuda.get_device_name(0))\nanswer')
  await page.getByRole('button', { name: 'Run cell' }).click()
  await page.waitForTimeout(900)
  await shot('09-console')

  await page.getByRole('tab', { name: 'Terminal' }).click()
  const terminal = page.getByRole('region', { name: 'Terminal' })
  await expect(terminal.getByText('root@colab:/content#').first()).toBeVisible()
  for (const command of ['nvidia-smi', 'ls']) {
    await page.locator('.xterm-helper-textarea').pressSequentially(command, { delay: 40 })
    await page.locator('.xterm-helper-textarea').press('Enter')
    await page.waitForTimeout(300)
  }
  await shot('10-terminal')

  await page.getByRole('tab', { name: 'Files' }).click()
  await page.getByRole('button', { name: 'nzap', exact: true }).click()
  await page.waitForTimeout(300)
  await page.getByRole('button', { name: 'outputs', exact: true }).click()
  await page.waitForTimeout(300)
  await shot('11-files')
})

test('updates (dark)', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('nzap.theme', 'dark')
    localStorage.setItem('nzap.updates.auto', 'off')
  })
  await openApp(page, { connected: true }, '/#/settings')
  await page.evaluate(() => {
    window.__NZAP_FAKE__!.state.update = {
      version: '0.2.0',
      body: 'Apps: one-click AI on your own Colab.\nA new look, and updates that install themselves.',
      date: '2026-10-20T00:00:00Z',
    }
  })
  await page.getByRole('button', { name: /Check for updates/ }).click()
  await expect(page.getByText('Version 0.2.0 is available')).toBeVisible()
  await page.waitForTimeout(300)
  await page.screenshot({ path: path.join(OUT, 'dark-14-updates.png') })
})
