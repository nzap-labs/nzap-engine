import { expect, fake, launchRuntime, openApp, runCell, test } from './fixtures'

test.describe('Console', () => {
  test('runs cells, answers input() and records history', async ({ page }) => {
    await openApp(page, { connected: true })
    await launchRuntime(page)

    await runCell(page, 'print("hello from the e2e")\nanswer')
    const output = page.locator('.font-mono.text-xs.leading-relaxed').first()
    await expect(output.getByText('hello from the e2e')).toBeVisible()
    await expect(output.getByText('42')).toBeVisible()
    await expect(output.getByText(/✓ finished/)).toBeVisible()

    await runCell(page, 'fail()')
    await expect(page.getByText('✗ finished with errors')).toBeVisible()
    await expect(page.getByText(/ValueError: boom/)).toBeVisible()

    await runCell(page, 'name = input("Name? ")')
    const answer = page.getByPlaceholder('type your answer and press Enter')
    await answer.fill('Grace')
    await answer.press('Enter')
    await expect(page.getByText('Hello, Grace!')).toBeVisible()

    const history = page.getByRole('region', { name: 'History' })
    await history.getByRole('button', { name: 'Refresh history' }).click()
    await expect(history.getByText(/events/)).not.toHaveText('0 events')
    await history.getByRole('button', { name: 'Notebook' }).click()
    await expect(page.getByText(/Saved to .*box\.ipynb/)).toBeVisible()
  })

  test('pauses for Drive consent and resumes after Continue', async ({ page }) => {
    await openApp(page, { connected: true, driveConsent: false })
    await launchRuntime(page)
    await runCell(page, "from google.colab import drive\ndrive.mount('/content/drive')")

    const approve = page.getByRole('link', { name: 'Approve access' }).first()
    await expect(approve).toBeVisible()
    await approve.click()
    expect(await fake(page, (state) => state.opened.at(-1))).toContain('accounts.google.com')

    await page.evaluate(() => {
      window.__NZAP_FAKE__!.state.driveConsent = true
    })
    await page.getByRole('button', { name: 'Continue' }).first().click()
    await expect(page.getByText('Mounted at /content/drive')).toBeVisible()
  })

  test('interrupts a long cell', async ({ page }) => {
    await openApp(page, { connected: true })
    await launchRuntime(page)
    await runCell(page, 'import time\ntime.sleep(3600)')
    await expect(page.getByRole('button', { name: 'Running…' })).toBeVisible()
    await page.getByRole('button', { name: 'Interrupt' }).click()
    await expect(page.getByRole('button', { name: 'Run cell' })).toBeEnabled()
  })

  test('installs packages from the setup card', async ({ page }) => {
    await openApp(page, { connected: true })
    await launchRuntime(page)
    const setup = page.getByRole('region', { name: 'Runtime setup' })
    await setup.getByPlaceholder(/torch transformers/).fill('numpy pandas')
    await setup.getByRole('button', { name: 'Install' }).click()
    await expect(setup.getByText('Installation Complete (via uv)!')).toBeVisible()
  })
})
