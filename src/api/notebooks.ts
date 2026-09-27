import { queryOptions, useMutation, useQueryClient } from '@tanstack/react-query'
import { call, stream, type StreamHandlers } from '@/lib/ipc'
import type { ColabExecuteEvent } from '@/types/colab'
import type { CatalogStatus, Notebook, NotebookParam, NotebookParamValues } from '@/types/notebook'

/**
 * Notebook library: the public collection (hosted on GitHub, verified and
 * cached by the engine) plus your own notebooks (local files).
 */
export const notebooksQuery = queryOptions({
  queryKey: ['notebooks'],
  queryFn: () => call<{ notebooks: Notebook[]; catalog: CatalogStatus }>('notebooks_list'),
  staleTime: 30_000,
})

export const notebookQuery = (id: string | null) =>
  queryOptions({
    queryKey: ['notebooks', id],
    queryFn: async () => ({ notebook: await call<Notebook>('notebook_get', { id }) }),
    enabled: Boolean(id),
    staleTime: 10_000,
  })

export interface NotebookDraft {
  slug: string
  title: string
  description: string
  source: string
  params: NotebookParam[]
  forkedFrom?: string | null
}

function useInvalidateNotebooks() {
  const queryClient = useQueryClient()
  return () => queryClient.invalidateQueries({ queryKey: ['notebooks'] })
}

/** Fetch (or revalidate) the public collection from GitHub. */
export function useRefreshCatalog() {
  const invalidate = useInvalidateNotebooks()
  return useMutation({
    mutationFn: () => call<CatalogStatus>('notebooks_refresh'),
    onSettled: () => invalidate(),
  })
}

export function useCreateNotebook() {
  const invalidate = useInvalidateNotebooks()
  return useMutation({
    mutationFn: (draft: NotebookDraft) => call<Notebook>('notebook_create', { draft }),
    onSuccess: () => invalidate(),
  })
}

export function useUpdateNotebook() {
  const invalidate = useInvalidateNotebooks()
  return useMutation({
    mutationFn: ({ id, draft }: { id: string; draft: Partial<NotebookDraft> }) =>
      call<Notebook>('notebook_update', { id, patch: draft }),
    onSuccess: () => invalidate(),
  })
}

export function useDeleteNotebook() {
  const invalidate = useInvalidateNotebooks()
  return useMutation({
    mutationFn: (id: string) => call<void>('notebook_delete', { id }),
    onSuccess: () => invalidate(),
  })
}

export function useExportNotebook() {
  return useMutation({
    mutationFn: (id: string) => call<string | null>('notebook_export', { id }),
  })
}

export function useImportNotebookFile() {
  const invalidate = useInvalidateNotebooks()
  return useMutation({
    mutationFn: () => call<Notebook | null>('notebook_import'),
    onSuccess: () => invalidate(),
  })
}

/**
 * Run a notebook on a connected runtime. The engine validates the values
 * against the declared parameters, injects them as `params`, and streams the
 * kernel's output — the same events the console gets.
 */
export async function streamNotebookRun(
  id: string,
  body: { session: string; params: NotebookParamValues },
  handlers: StreamHandlers<ColabExecuteEvent>,
): Promise<void> {
  await stream('notebook_run', { id, session: body.session, params: body.params }, handlers)
}
