import { test as base, expect, type Page } from '@playwright/test'
import type { FakeState } from '../../src/dev/fake-engine'

type Preset = Partial<Pick<FakeState, 'connected' | 'driveConsent' | 'delay' | 'failNextCreate'>>

/** Open the app with the simulated engine in a given starting state. */
export async function openApp(page: Page, preset: Preset = {}, path = '/') {
  await page.addInitScript(
    (state) => {
      sessionStorage.setItem('nzap-fake-preset', JSON.stringify(state))
    },
    { delay: 30, ...preset },
  )
  await page.goto(path)
  await page.waitForFunction(() => Boolean(window.__NZAP_FAKE__))
}

/** Read the simulated engine's state (calls, opened URLs, saved files…). */
export function fake<T>(page: Page, read: (state: FakeState) => T): Promise<T> {
  return page.evaluate((source) => {
    const reader = new Function('state', `return (${source})(state)`) as (state: unknown) => unknown
    return reader(window.__NZAP_FAKE__!.state)
  }, read.toString()) as Promise<T>
}

/** Launch a runtime from the Runtimes tab and wait until it is ready. */
export async function launchRuntime(
  page: Page,
  name = 'box',
  hardware: 'cpu' | 'gpu' | 'tpu' = 'cpu',
) {
  await page.getByRole('tab', { name: 'Runtimes' }).click()
  await page.getByPlaceholder('my-runtime').fill(name)
  await page.getByRole('radio', { name: hardware }).click()
  await page.getByRole('button', { name: 'Launch runtime' }).click()
  await expect(page.getByText(`Runtime ${name} is ready`)).toBeVisible()
}

export async function runCell(page: Page, code: string) {
  const editor = page.getByLabel('Code')
  await editor.fill(code)
  await page.getByRole('button', { name: 'Run cell' }).click()
}

export const test = base
export { expect }
