import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { renderWithEngine } from '@/test/render'
import { NotebooksPanel } from './notebooks-panel'

describe('NotebooksPanel', () => {
  it('lists the public collection and runs a notebook', async () => {
    const onRun = vi.fn()
    const { user } = renderWithEngine(<NotebooksPanel onRun={onRun} canRun runtimeName="box" />, {
      connected: true,
    })
    const collection = await screen.findByRole('region', { name: 'Public collection' })
    expect(within(collection).getByText('built-in copy')).toBeInTheDocument()
    const print = within(collection).getByText('Print Notebook').closest('li')!
    expect(print).toHaveTextContent('1 parameter')
    await user.click(within(print).getByRole('button', { name: /Run/ }))
    expect(onRun).toHaveBeenCalledWith(expect.objectContaining({ id: 'public:print-notebook' }))
  })

  it('refreshes the collection from GitHub', async () => {
    const { user, engine } = renderWithEngine(
      <NotebooksPanel onRun={vi.fn()} canRun={false} runtimeName={null} />,
    )
    await user.click(await screen.findByRole('button', { name: /Refresh/ }))
    await waitFor(() => expect(engine.state.calls).toContain('notebooks_refresh'))
  })

  it('forks with the full source loaded into the editor', async () => {
    const { user, engine } = renderWithEngine(
      <NotebooksPanel onRun={vi.fn()} canRun={false} runtimeName={null} />,
    )
    const collection = await screen.findByRole('region', { name: 'Public collection' })
    const print = within(collection).getByText('Print Notebook').closest('li')!
    await user.click(within(print).getByRole('button', { name: /Fork/ }))

    const editor = await screen.findByRole('dialog')
    // List entries carry no source: the editor must load it.
    expect(
      within(editor).getByDisplayValue(/print\(params\["string_to_print"\]\)/),
    ).toBeInTheDocument()
    await user.type(within(editor).getByLabelText(/Slug/i), 'my-print')
    await user.click(within(editor).getByRole('button', { name: /Create notebook/ }))

    await waitFor(() =>
      expect(engine.state.notebooks.some((notebook) => notebook.slug === 'my-print')).toBe(true),
    )
    const mine = engine.state.notebooks.find((notebook) => notebook.slug === 'my-print')!
    expect(mine.forkedFrom).toBe('print-notebook')
    expect(await screen.findByText(/forked from print-notebook/)).toBeInTheDocument()
  })

  it('deletes your notebooks after confirmation, never public ones', async () => {
    const { user, engine } = renderWithEngine(
      <NotebooksPanel onRun={vi.fn()} canRun={false} runtimeName={null} />,
      {},
      (state) =>
        state.notebooks.push({
          id: 'local:abc',
          slug: 'scratch',
          title: 'Scratch',
          description: '',
          source: 'print(1)',
          params: [],
          tags: [],
          author: null,
          visibility: 'private',
          createdAt: null,
          updatedAt: null,
          forkedFrom: null,
        }),
    )
    const yours = await screen.findByRole('region', { name: 'Your notebooks' })
    await within(yours).findByText('Scratch')
    const publicSection = screen.getByRole('region', { name: 'Public collection' })
    expect(within(publicSection).queryByRole('button', { name: /Delete/ })).not.toBeInTheDocument()

    await user.click(within(yours).getByRole('button', { name: /Delete/ }))
    await user.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Delete' }))
    await waitFor(() =>
      expect(engine.state.notebooks.some((notebook) => notebook.id === 'local:abc')).toBe(false),
    )
  })
})
