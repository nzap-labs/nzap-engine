import { expect, openApp, test } from './fixtures'

test.describe('Keyboard access', () => {
  test('workspace tabs follow the WAI-ARIA tabs pattern', async ({ page }) => {
    await openApp(page, { connected: true })
    const runtimes = page.getByRole('tab', { name: 'Runtimes' })
    await expect(runtimes).toHaveAttribute('aria-selected', 'true')
    await expect(runtimes).toHaveAttribute('tabindex', '0')
    await expect(page.getByRole('tab', { name: 'Files' })).toHaveAttribute('tabindex', '-1')
    await expect(page.getByRole('tabpanel', { name: 'Runtimes' })).toBeVisible()

    await runtimes.focus()
    await page.keyboard.press('ArrowRight')
    const console = page.getByRole('tab', { name: 'Console' })
    await expect(console).toBeFocused()
    await expect(console).toHaveAttribute('aria-selected', 'true')
    await expect(page.getByRole('tabpanel', { name: 'Console' })).toBeVisible()

    await page.keyboard.press('End')
    await expect(page.getByRole('tab', { name: 'Files' })).toBeFocused()
    await page.keyboard.press('ArrowRight')
    await expect(runtimes).toBeFocused()
    await page.keyboard.press('ArrowLeft')
    await expect(page.getByRole('tab', { name: 'Files' })).toBeFocused()
    await page.keyboard.press('Home')
    await expect(runtimes).toBeFocused()
    await expect(page.getByRole('tabpanel', { name: 'Runtimes' })).toBeVisible()
  })

  test('the account menu and settings work from the keyboard', async ({ page }) => {
    await openApp(page, { connected: true })
    await page.getByRole('button', { name: /Google connected/ }).focus()
    await page.keyboard.press('Enter')
    const settings = page.getByRole('menuitem', { name: 'Settings' })
    await expect(settings).toBeVisible()
    await settings.focus()
    await page.keyboard.press('Enter')
    await expect(page.getByRole('heading', { name: 'Settings' })).toBeVisible()

    const tray = page.getByRole('checkbox', {
      name: 'Keep running in the system tray when the window is closed',
    })
    await tray.focus()
    await page.keyboard.press('Space')
    await expect(tray).toBeChecked()
  })
})
