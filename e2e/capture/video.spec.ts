import { mkdirSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import type { Locator, Page } from '@playwright/test'
import { expect, openApp, test } from '../web/fixtures'

/**
 * Shots for the launch video (nzap-website/video): dark theme, 1440×900 at
 * 2×, plus `boxes.json` with the CSS-pixel rectangle of every element the
 * video's camera zooms into or its cursor clicks.
 */
const OUT = path.join(process.env.CAPTURE_DIR ?? path.resolve('e2e/capture/out'), 'video')
mkdirSync(OUT, { recursive: true })

type Box = { x: number; y: number; width: number; height: number }
const boxes: Record<string, Record<string, Box>> = {}

async function scrollTop(page: Page) {
  await page.evaluate(() =>
    document.querySelectorAll('.overflow-y-auto').forEach((element) => element.scrollTo(0, 0)),
  )
}

async function shot(page: Page, name: string, targets: Record<string, Locator> = {}) {
  await page.waitForTimeout(350)
  boxes[name] = {}
  for (const [key, locator] of Object.entries(targets)) {
    const box = await locator.first().boundingBox()
    if (box) boxes[name][key] = box
  }
  await page.screenshot({ path: path.join(OUT, `${name}.png`) })
}

test('video shots', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('nzap.theme', 'dark')
    localStorage.setItem('nzap.updates.auto', 'off')
    // The film animates its own callouts; app toasts would clutter the shots.
    document.addEventListener('DOMContentLoaded', () => {
      const style = document.createElement('style')
      style.textContent = '[data-sonner-toaster]{display:none!important}'
      document.head.append(style)
    })
  })

  await openApp(page, { connected: false, delay: 30 }, '/#/colab')
  await expect(page.getByRole('button', { name: 'Connect Google' })).toBeVisible()
  await shot(page, 'connect', {
    connect: page.getByRole('button', { name: 'Connect Google' }),
    welcome: page.getByRole('region', { name: 'Welcome' }),
  })
  await page.getByRole('button', { name: 'Connect Google' }).click()
  await expect(page.getByText('Google Auth connected')).toBeVisible()
  await shot(page, 'connected', {
    card: page.getByText(/Connected as/).locator('xpath=ancestor::section[1]'),
    keychain: page.getByText('in system keychain'),
  })

  await page.goto('/#/apps')
  const kokoro = page.getByRole('link', { name: /Kokoro Text to Speech/ })
  await expect(kokoro).toBeVisible()
  await shot(page, 'apps', {
    kokoro,
    breeze: page.getByRole('link', { name: /Breeze TTS 2/ }),
    grid: page.getByRole('list').filter({ has: kokoro }),
    hero: page.getByRole('heading', { name: /One click from model to result/ }),
  })

  await kokoro.click()
  await page.getByRole('button', { name: 'Product intro' }).click()
  await scrollTop(page)
  await shot(page, 'form', {
    chips: page.getByText(/Best on T4/).locator('xpath=..'),
    best: page.getByText(/Best on T4/),
    timing: page.getByText(/Setup ~50s/),
    runtime: page.getByText(/No runtime yet/),
    example: page.getByRole('button', { name: 'Product intro' }),
    text: page.getByLabel(/Text/),
    inputs: page.getByRole('region', { name: 'Inputs' }),
  })

  // The bottom of the form, with the run button and its estimate.
  const generate = page.getByRole('button', { name: /Generate speech/ })
  await generate.scrollIntoViewIfNeeded()
  await shot(page, 'form-bottom', {
    generate,
    estimate: page.getByText(/on T4.*setup/),
    voice: page.getByRole('combobox'),
    language: page.getByRole('radiogroup', { name: 'Language' }),
  })

  await generate.click()
  await expect(page.getByText(/Installing|Downloading/).first()).toBeVisible()
  await scrollTop(page)
  await shot(page, 'running', {
    results: page.getByRole('region', { name: 'Results' }),
    phases: page.getByRole('region', { name: 'Results' }).getByRole('list').first(),
  })

  await expect(page.getByRole('button', { name: 'Play' })).toBeVisible({ timeout: 20_000 })
  await scrollTop(page)
  await shot(page, 'result', {
    results: page.getByRole('region', { name: 'Results' }),
    player: page.getByRole('slider', { name: 'Seek' }).locator('xpath=../..'),
    play: page.getByRole('button', { name: 'Play' }),
    waveform: page.getByRole('slider', { name: 'Seek' }),
  })

  await page.getByRole('button', { name: /Generate speech/ }).click()
  await expect(page.getByText('Already warm')).toBeVisible()
  await scrollTop(page)
  await shot(page, 'warm', {
    warm: page.getByText('Already warm').locator('xpath=ancestor::li[1]'),
    runtime: page.getByRole('combobox').first(),
    phases: page.getByRole('region', { name: 'Results' }).getByRole('list').first(),
  })
  await expect(page.getByText(/Earlier results/)).toBeVisible()
  await scrollTop(page)
  await shot(page, 'warm-done', {
    results: page.getByRole('region', { name: 'Results' }),
    play: page.getByRole('button', { name: 'Play' }).first(),
  })

  await page.goto('/#/colab')
  await page.getByRole('region', { name: 'Runtimes' }).getByText('app-kokoro-tts').first().click()
  await page.getByLabel('Code').fill('import torch\nprint(torch.cuda.get_device_name(0))\nanswer')
  await page.getByRole('button', { name: 'Run cell' }).click()
  await page.waitForTimeout(800)
  await shot(page, 'console')

  await page.getByRole('tab', { name: 'Terminal' }).click()
  await expect(page.getByText('root@colab:/content#').first()).toBeVisible()
  await page.locator('.xterm-helper-textarea').pressSequentially('nvidia-smi', { delay: 20 })
  await page.locator('.xterm-helper-textarea').press('Enter')
  await shot(page, 'terminal', { terminal: page.getByRole('region', { name: 'Terminal' }) })

  await page.getByRole('tab', { name: 'Files' }).click()
  await page.getByRole('button', { name: 'nzap', exact: true }).click()
  await page.waitForTimeout(250)
  await page.getByRole('button', { name: 'outputs', exact: true }).click()
  await shot(page, 'files', { files: page.getByRole('region', { name: 'Files' }) })

  await page.getByRole('tab', { name: 'Notebooks' }).click()
  await shot(page, 'notebooks')

  await page.getByRole('tab', { name: 'Runtimes' }).click()
  await shot(page, 'runtimes')

  writeFileSync(path.join(OUT, 'boxes.json'), JSON.stringify(boxes, null, 2))
})
