import { fireEvent, screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { call } from '@/lib/ipc'
import { renderWithEngine } from '@/test/render'
import { FilesPanel } from './files-panel'

async function setup() {
  const rendered = renderWithEngine(<FilesPanel sessionName="box" />, { connected: true })
  await call('session_create', { request: { name: 'box' } })
  await rendered.queryClient.invalidateQueries()
  return rendered
}

describe('FilesPanel', () => {
  it('browses folders', async () => {
    const { user } = await setup()
    // Opens in /content, like Colab's file browser.
    await user.click(await screen.findByRole('button', { name: /^sample_data$/ }))
    expect(await screen.findByText('README.md')).toBeInTheDocument()
    const breadcrumb = screen.getByRole('navigation', { name: 'File path' })
    expect(breadcrumb).toHaveTextContent('content')
    expect(breadcrumb).toHaveTextContent('sample_data')
    await user.click(within(breadcrumb).getByRole('button', { name: 'Root folder' }))
    expect(await screen.findByRole('button', { name: /^content/ })).toBeInTheDocument()
  })

  it('creates a folder through the in-app prompt', async () => {
    const { user, engine } = await setup()
    await user.click(await screen.findByRole('button', { name: /New folder/ }))
    const dialog = screen.getByRole('dialog', { name: 'New folder' })
    await user.type(within(dialog).getByRole('textbox'), 'datasets{Enter}')
    await waitFor(() =>
      expect(engine.state.sessions.get('box')!.files.get('content/datasets')?.dir).toBe(true),
    )
  })

  it('uploads picked and dropped files', async () => {
    const { user, engine } = await setup()
    const input = document.querySelector('input[type="file"]') as HTMLInputElement
    await user.upload(input, new File(['a,b\n1,2\n'], 'data.csv', { type: 'text/csv' }))
    await waitFor(() =>
      expect(engine.state.sessions.get('box')!.files.get('content/data.csv')?.content).toBe(
        'a,b\n1,2\n',
      ),
    )

    const dropped = new File(['print(1)\n'], 'drop.py')
    fireEvent.drop(screen.getByRole('region', { name: 'Files' }), {
      dataTransfer: { files: [dropped] },
    })
    await waitFor(() =>
      expect(engine.state.sessions.get('box')!.files.get('content/drop.py')?.content).toBe(
        'print(1)\n',
      ),
    )
  })

  it('edits and saves a text file', async () => {
    const { user, engine } = await setup()
    await user.click(await screen.findByRole('button', { name: /^sample_data$/ }))
    await user.click(await screen.findByRole('button', { name: /^README\.md/ }))
    const editor = await screen.findByDisplayValue('Sample datasets.')
    await user.clear(editor)
    await user.type(editor, 'Edited.')
    await user.click(screen.getByRole('button', { name: /Save/ }))
    await waitFor(() =>
      expect(
        engine.state.sessions.get('box')!.files.get('content/sample_data/README.md')?.content,
      ).toBe('Edited.'),
    )
  })
})
