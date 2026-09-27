import { expect, fake, openApp, test } from './fixtures'

test.describe('Runtimes', () => {
  test.beforeEach(async ({ page }) => {
    await openApp(page, { connected: true })
  })

  test('launches a GPU runtime; accelerators the plan lacks are disabled', async ({ page }) => {
    await page.getByPlaceholder('my-runtime').fill('trainer')
    await page.getByRole('radio', { name: 'gpu' }).click()
    const accelerator = page.getByRole('combobox')
    await expect(accelerator).toHaveValue('t4')
    await expect(accelerator.locator('option[value="a100"]')).toHaveAttribute('disabled', '')
    await page.getByRole('checkbox', { name: 'High-RAM shape' }).check()
    await page.getByRole('button', { name: 'Launch runtime' }).click()

    await expect(page.getByText('Runtime trainer is ready (T4).')).toBeVisible()
    // The app switches to the console for the new runtime.
    await expect(page.getByRole('tab', { name: 'Console' })).toHaveAttribute(
      'aria-selected',
      'true',
    )
    await page.getByRole('tab', { name: 'Runtimes' }).click()
    const card = page.getByRole('region', { name: 'Runtimes' })
    await expect(card.getByText('trainer')).toBeVisible()
    await expect(card.getByText('High-RAM')).toBeVisible()
    await expect(card.getByText(/of lifetime left/)).toBeVisible()
  })

  test('keep-alive, restart and stop', async ({ page }) => {
    await page.getByPlaceholder('my-runtime').fill('box')
    await page.getByRole('button', { name: 'Launch runtime' }).click()
    await expect(page.getByText('Runtime box is ready')).toBeVisible()
    await page.getByRole('tab', { name: 'Runtimes' }).click()
    const card = page.getByRole('region', { name: 'Runtimes' })

    await card.getByRole('button', { name: 'Keep alive' }).click()
    await expect(page.getByText('keepalive → box')).toBeVisible()
    await card.getByRole('button', { name: 'Restart' }).click()
    await expect(page.getByText('restart → box')).toBeVisible()

    // Opening Colab goes through the engine to the system browser.
    await card.getByRole('link', { name: 'Open in Colab' }).click()
    expect(await fake(page, (state) => state.opened.at(-1))).toContain('colab.research.google.com')

    await card.getByRole('button', { name: 'Stop' }).click()
    await page
      .getByRole('dialog', { name: 'Stop box?' })
      .getByRole('button', { name: 'Stop and release' })
      .click()
    await expect(page.getByText('Released box.')).toBeVisible()
    await expect(card.getByText('No runtimes yet')).toBeVisible()
  })

  test('imports and releases runtimes created elsewhere', async ({ page }) => {
    const external = page.getByRole('region', { name: 'External runtimes' })
    await expect(external.getByText('m-s-t4-webui')).toBeVisible()
    await external.getByRole('button', { name: 'Import' }).click()
    await expect(page.getByText('Runtime imported.')).toBeVisible()
    await expect(
      page.getByRole('region', { name: 'Runtimes' }).getByText('imported-m-s-t4-w'),
    ).toBeVisible()
  })

  test('explains allocation refusals', async ({ page }) => {
    await openApp(page, {
      connected: true,
      failNextCreate: {
        code: 'too_many_runtimes',
        message:
          'This account already holds the maximum number of Colab VMs. Release one of the existing runtimes first.',
      },
    })
    await page.getByRole('button', { name: 'Launch runtime' }).click()
    await expect(page.getByText(/maximum number of Colab VMs/)).toBeVisible()
  })
})
