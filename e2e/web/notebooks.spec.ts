import { expect, fake, launchRuntime, openApp, test } from './fixtures'

test.describe('Notebooks', () => {
  test('runs a public notebook with parameters', async ({ page }) => {
    await openApp(page, { connected: true })
    await launchRuntime(page)
    await page.getByRole('tab', { name: 'Notebooks' }).click()

    const card = page.getByRole('listitem').filter({ hasText: 'Print Notebook' })
    await card.getByRole('button', { name: 'Run' }).click()
    const dialog = page.getByRole('dialog', { name: 'Print Notebook' })
    await dialog.getByLabel(/String to print/).fill('Hi from a notebook')
    await dialog.getByRole('button', { name: 'Run', exact: true }).click()
    await expect(dialog.getByText('Hi from a notebook')).toBeVisible()
  })

  test('forks a public notebook, edits it and deletes it', async ({ page }) => {
    await openApp(page, { connected: true })
    await page.getByRole('tab', { name: 'Notebooks' }).click()

    await page
      .getByRole('listitem')
      .filter({ hasText: 'Print Notebook' })
      .getByRole('button', { name: 'Fork' })
      .click()
    const editor = page.getByRole('dialog')
    // The editor loads the full source (list entries carry none).
    await expect(editor.getByLabel(/Source/)).toHaveValue(/params\["string_to_print"\]/)
    await editor.getByLabel(/Slug/).fill('my-print')
    await editor.getByRole('button', { name: 'Create notebook' }).click()
    await expect(page.getByText('Notebook saved to your collection.')).toBeVisible()

    const yours = page.getByRole('region', { name: 'Your notebooks' })
    await expect(yours.getByText('forked from print-notebook')).toBeVisible()

    await yours.getByRole('button', { name: 'Edit' }).click()
    await page.getByRole('dialog').getByLabel(/Title/).fill('My Printer')
    await page.getByRole('dialog').getByRole('button', { name: 'Save changes' }).click()
    await expect(yours.getByText('My Printer')).toBeVisible()

    await yours.getByRole('button', { name: /Export My Printer/ }).click()
    await expect(page.getByText(/Saved to .*my-print\.nzap\.json/)).toBeVisible()

    await yours.getByRole('button', { name: 'Delete' }).click()
    await page
      .getByRole('dialog', { name: 'Delete My Printer?' })
      .getByRole('button', { name: 'Delete' })
      .click()
    await expect(yours.getByText('You have no notebooks yet')).toBeVisible()
    expect(
      await fake(
        page,
        (state) => state.notebooks.filter((notebook) => notebook.visibility === 'private').length,
      ),
    ).toBe(0)
  })
})

test.describe('Account and settings', () => {
  test('shows the Colab plan and compute units', async ({ page }) => {
    await openApp(page, { connected: true }, '/#/account')
    const compute = page.getByRole('region', { name: 'Colab compute' })
    await expect(compute.getByText('Free', { exact: true })).toBeVisible()
    await expect(compute.getByText('Available accelerators:')).toBeVisible()
    await expect(page.getByText('ada@example.com').first()).toBeVisible()
  })

  test('validates and saves settings', async ({ page }) => {
    await openApp(page, { connected: true }, '/#/settings')
    const catalog = page.getByRole('region', { name: 'Public notebook collection' })
    await catalog.getByLabel('Catalog URL').fill('http://example.com/catalog/')
    await catalog.getByRole('button', { name: 'Save' }).click()
    await expect(page.getByText('The catalog URL must be an https:// address.')).toBeVisible()

    const keepAlive = page.getByRole('region', { name: 'Keep-alive' })
    await keepAlive.getByRole('spinbutton').fill('120')
    await keepAlive.getByRole('button', { name: 'Save' }).click()
    await expect(page.getByText('Settings saved.')).toBeVisible()
    expect(await fake(page, (state) => state.settings.keepAliveIntervalSeconds)).toBe(120)

    // AI agents get a ready-to-paste MCP command for this installation.
    const agents = page.getByRole('region', { name: 'AI agents (MCP)' })
    await expect(agents.getByText(/^claude mcp add --scope user nzap -- /)).toBeVisible()
    await expect(agents.getByText(/"mcpServers"/)).toBeVisible()

    // Changing the OAuth client needs Google disconnected first.
    const client = page.getByRole('region', { name: 'Google OAuth client' })
    await client.getByLabel('OAuth client JSON').fill('{"installed": {"client_id": "mine"}}')
    await client.getByRole('button', { name: 'Use this client' }).click()
    await expect(
      page.getByText('Disconnect Google before changing the OAuth client.'),
    ).toBeVisible()
  })

  test('navigation: chat workspace, sidebar and unknown pages', async ({ page }) => {
    await openApp(page, { connected: true })
    await page.getByRole('link', { name: 'Chat' }).click()
    await expect(page.getByRole('heading', { name: 'What can I build for you?' })).toBeVisible()

    await page.getByRole('button', { name: 'Close sidebar' }).click()
    await page.getByRole('button', { name: 'Open sidebar' }).click()
    await expect(page.getByRole('link', { name: 'Colab', exact: true })).toBeVisible()

    await page.goto('/#/nowhere')
    await expect(page.getByText('Page not found')).toBeVisible()
    await page.getByRole('link', { name: 'Back to Colab' }).click()
    await expect(page.getByText('Google Auth connected')).toBeVisible()
  })
})
