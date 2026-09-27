import { expect, fake, launchRuntime, openApp, test } from './fixtures'

test.describe('Terminal, files, run and jobs', () => {
  test.beforeEach(async ({ page }) => {
    await openApp(page, { connected: true })
    await launchRuntime(page)
  })

  test('a shell on the runtime', async ({ page }) => {
    await page.getByRole('tab', { name: 'Terminal' }).click()
    const terminal = page.getByRole('region', { name: 'Terminal' })
    await expect(terminal.getByText('root@fake:/content#').first()).toBeVisible()
    await page.locator('.xterm-helper-textarea').pressSequentially('whoami')
    await page.locator('.xterm-helper-textarea').press('Enter')
    await expect(terminal.getByText(/^root$/)).toBeVisible()
  })

  test('file manager: browse, create, upload, rename, delete', async ({ page }) => {
    await page.getByRole('tab', { name: 'Files' }).click()
    const files = page.getByRole('region', { name: 'Files' })
    await expect(files.getByRole('button', { name: 'sample_data', exact: true })).toBeVisible()

    await files.getByRole('button', { name: 'New folder' }).click()
    await page.getByRole('dialog', { name: 'New folder' }).getByRole('textbox').fill('datasets')
    await page
      .getByRole('dialog', { name: 'New folder' })
      .getByRole('button', { name: 'OK' })
      .click()
    await expect(files.getByRole('button', { name: 'datasets', exact: true })).toBeVisible()

    await files.locator('input[type="file"]').setInputFiles({
      name: 'train.py',
      mimeType: 'text/x-python',
      buffer: Buffer.from('print("train")\n'),
    })
    await expect(page.getByText('Uploaded train.py.')).toBeVisible()

    await files.getByRole('button', { name: 'Rename train.py' }).click()
    const rename = page.getByRole('dialog', { name: 'Rename train.py' })
    await rename.getByRole('textbox').fill('fit.py')
    await rename.getByRole('button', { name: 'OK' }).click()
    await expect(files.getByRole('button', { name: /^fit\.py/ })).toBeVisible()

    await files.getByRole('button', { name: 'Delete fit.py' }).click()
    await page
      .getByRole('dialog', { name: 'Delete fit.py?' })
      .getByRole('button', { name: 'Delete' })
      .click()
    await expect(files.getByRole('button', { name: /^fit\.py/ })).toHaveCount(0)
    expect(await fake(page, (state) => [...state.sessions.get('box')!.files.keys()])).toContain(
      'content/datasets',
    )
  })

  test('runs a notebook file and saves the executed copy', async ({ page }) => {
    await page.getByRole('tab', { name: 'Run', exact: true }).click()
    const notebook = {
      cells: [
        { cell_type: 'markdown', source: '# Experiment' },
        { cell_type: 'code', source: 'print("cell one")' },
        { cell_type: 'code', source: 'print("cell two")' },
      ],
      nbformat: 4,
    }
    await page.locator('input[type="file"][accept*="ipynb"]').setInputFiles({
      name: 'exp.ipynb',
      mimeType: 'application/json',
      buffer: Buffer.from(JSON.stringify(notebook)),
    })
    await page
      .getByRole('region', { name: 'Run a file' })
      .getByRole('button', { name: 'Run', exact: true })
      .click()
    await expect(page.getByText('cell two')).toBeVisible()
    await page.getByRole('button', { name: 'exp_output.ipynb' }).click()
    await expect(page.getByText(/Saved to .*exp_output\.ipynb/)).toBeVisible()
    const saved = await fake(page, (state) => state.saved.at(-1)!)
    expect(JSON.parse(saved.content).cells[2].outputs[0].text).toBe('cell two\n')
  })

  test('an ephemeral job returns artifacts and releases its VM', async ({ page }) => {
    await page.getByRole('tab', { name: 'Run', exact: true }).click()
    const jobs = page.getByRole('region', { name: 'Jobs' })
    await jobs
      .getByLabel('Script', { exact: true })
      .fill('print("training")\nimport sys\nsys.exit(0)')
    await jobs.getByPlaceholder(/out\/\*\.png/).fill('out/*')
    await jobs.getByRole('button', { name: 'Run job' }).click()
    await expect(jobs.getByText('Exit code 0 · VM released')).toBeVisible()
    await expect(jobs.getByText('/content/out/result.txt')).toBeVisible()
    await expect(jobs.getByText('training', { exact: true })).toBeVisible()
  })
})
