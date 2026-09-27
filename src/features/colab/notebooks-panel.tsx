import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import {
  BookOpen,
  Copy,
  Download,
  FileCode2,
  FileUp,
  Lock,
  Pencil,
  Play,
  Plus,
  RefreshCw,
  Trash2,
  Users,
} from 'lucide-react'
import { toast } from 'sonner'
import {
  notebookQuery,
  notebooksQuery,
  useDeleteNotebook,
  useExportNotebook,
  useImportNotebookFile,
  useRefreshCatalog,
} from '@/api/notebooks'
import { useDialogs } from '@/components/dialogs'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/cn'
import { errorMessage } from '@/lib/ipc'
import type { CatalogStatus, Notebook } from '@/types/notebook'
import { NotebookEditorDialog } from './notebook-editor-dialog'

type EditorState =
  | { mode: 'create' }
  | { mode: 'edit'; notebook: Notebook }
  | { mode: 'fork'; notebook: Notebook }
  | null

const ORIGIN_LABELS: Record<CatalogStatus['origin'], string> = {
  remote: 'up to date',
  cache: 'saved copy',
  bundled: 'built-in copy',
}

/**
 * The notebook library: the public collection from GitHub plus your own
 * notebooks. Running one needs a connected runtime, so the run button is only
 * offered when there is one.
 */
export function NotebooksPanel({
  onRun,
  canRun,
  runtimeName,
}: {
  onRun: (notebook: Notebook) => void
  canRun: boolean
  runtimeName: string | null
}) {
  const queryClient = useQueryClient()
  const { data, isPending, error } = useQuery(notebooksQuery)
  const [editor, setEditor] = useState<EditorState>(null)
  const remove = useDeleteNotebook()
  const refresh = useRefreshCatalog()
  const exportNotebook = useExportNotebook()
  const importFile = useImportNotebookFile()
  const dialogs = useDialogs()

  /** List entries carry no source; load the full notebook before editing. */
  async function openEditor(mode: 'edit' | 'fork', summary: Notebook) {
    try {
      const { notebook } = await queryClient.fetchQuery(notebookQuery(summary.id))
      setEditor({ mode, notebook })
    } catch (loadError) {
      toast.error(errorMessage(loadError, 'Could not open that notebook.'))
    }
  }

  if (isPending) {
    return (
      <section className="rounded-[24px] border border-ink bg-paper p-6">
        <div className="h-6 w-40 animate-pulse rounded bg-paper-soft" />
        <div className="mt-4 space-y-3">
          <div className="h-20 animate-pulse rounded-2xl bg-paper-soft" />
          <div className="h-20 animate-pulse rounded-2xl bg-paper-soft" />
        </div>
      </section>
    )
  }

  if (error) {
    return (
      <section className="rounded-[24px] border border-ink bg-paper p-6">
        <p className="font-medium">Notebooks</p>
        <p className="mt-2 text-sm text-graphite">
          Could not load the library: {errorMessage(error, 'unknown error')}.
        </p>
      </section>
    )
  }

  const notebooks = data?.notebooks ?? []
  const catalog = data?.catalog
  const publicNotebooks = notebooks.filter((notebook) => notebook.visibility === 'public')
  const mine = notebooks.filter((notebook) => notebook.isMine)

  return (
    <div className="space-y-6">
      <section
        aria-label="Public collection"
        className="rounded-[24px] border border-ink bg-paper p-6"
      >
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex items-center gap-2">
            <Users className="size-4 text-graphite" />
            <p className="font-medium">Public collection</p>
            {catalog && (
              <span
                className={cn(
                  'rounded-full border px-2 py-0.5 text-[11px] font-medium uppercase tracking-wide',
                  catalog.origin === 'remote' ? 'border-mint' : 'border-line text-graphite',
                )}
                title={catalog.error ?? catalog.url}
              >
                {ORIGIN_LABELS[catalog.origin]}
              </span>
            )}
          </div>
          <Button
            variant="ghost"
            size="sm"
            disabled={refresh.isPending}
            onClick={() =>
              refresh.mutate(undefined, {
                onSuccess: (status) =>
                  status.error
                    ? toast.warning(`Using the ${ORIGIN_LABELS[status.origin]}: ${status.error}`)
                    : toast.success(`Collection updated (${status.count} notebooks).`),
              })
            }
          >
            <RefreshCw className={cn('size-4', refresh.isPending && 'animate-spin')} /> Refresh
          </Button>
        </div>
        <p className="mt-1 text-sm text-graphite">
          Community notebooks from GitHub, checked against their published SHA-256 before they run.
          They run on your own runtime — nothing leaves your account.
        </p>
        <ul className="mt-4 space-y-3">
          {publicNotebooks.map((notebook) => (
            <NotebookCard
              key={notebook.id}
              notebook={notebook}
              onRun={onRun}
              canRun={canRun}
              runtimeName={runtimeName}
              onFork={() => void openEditor('fork', notebook)}
            />
          ))}
          {publicNotebooks.length === 0 && (
            <li className="text-sm text-graphite">No public notebooks yet.</li>
          )}
        </ul>
      </section>

      <section
        aria-label="Your notebooks"
        className="rounded-[24px] border border-line bg-paper p-6"
      >
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex items-center gap-2">
            <Lock className="size-4 text-graphite" />
            <p className="font-medium">Your notebooks</p>
          </div>
          <div className="flex flex-wrap gap-2">
            <Button
              variant="ghost"
              size="sm"
              disabled={importFile.isPending}
              onClick={() =>
                importFile.mutate(undefined, {
                  onSuccess: (notebook) => notebook && toast.success(`Imported ${notebook.title}.`),
                  onError: (importError) =>
                    toast.error(errorMessage(importError, 'Import failed.')),
                })
              }
            >
              <FileUp className="size-4" /> Import
            </Button>
            <Button variant="secondary" size="sm" onClick={() => setEditor({ mode: 'create' })}>
              <Plus className="size-4" /> New notebook
            </Button>
          </div>
        </div>
        <p className="mt-1 text-sm text-graphite">
          Stored on this computer. Fork a public one or write your own — the editor opens with a
          starter script.
        </p>
        <ul className="mt-4 space-y-3">
          {mine.map((notebook) => (
            <NotebookCard
              key={notebook.id}
              notebook={notebook}
              onRun={onRun}
              canRun={canRun}
              runtimeName={runtimeName}
              onEdit={() => void openEditor('edit', notebook)}
              onExport={() =>
                exportNotebook.mutate(notebook.id, {
                  onSuccess: (path) => path && toast.success(`Saved to ${path}.`),
                  onError: (exportError) =>
                    toast.error(errorMessage(exportError, 'Export failed.')),
                })
              }
              onDelete={async () => {
                const confirmed = await dialogs.confirm({
                  title: `Delete ${notebook.title}?`,
                  description: 'The notebook is removed from this computer.',
                  confirmLabel: 'Delete',
                  danger: true,
                })
                if (!confirmed) return
                remove.mutate(notebook.id, {
                  onSuccess: () => toast.success(`Deleted ${notebook.title}.`),
                  onError: (deleteError) =>
                    toast.error(errorMessage(deleteError, 'Delete failed.')),
                })
              }}
            />
          ))}
          {mine.length === 0 && (
            <li className="text-sm text-graphite">
              You have no notebooks yet — they show up here once you create one.
            </li>
          )}
        </ul>
      </section>

      {editor && editor.mode === 'create' && (
        <NotebookEditorDialog mode="create" onClose={() => setEditor(null)} />
      )}
      {editor && editor.mode !== 'create' && (
        <NotebookEditorDialog
          mode={editor.mode}
          notebook={editor.notebook}
          onClose={() => setEditor(null)}
        />
      )}
    </div>
  )
}

function NotebookCard({
  notebook,
  onRun,
  canRun,
  runtimeName,
  onEdit,
  onFork,
  onExport,
  onDelete,
}: {
  notebook: Notebook
  onRun: (notebook: Notebook) => void
  canRun: boolean
  runtimeName: string | null
  onEdit?: () => void
  onFork?: () => void
  onExport?: () => void
  onDelete?: () => void
}) {
  const paramCount = notebook.params.length

  return (
    <li className="flex flex-wrap items-center justify-between gap-4 rounded-2xl border border-line bg-paper-soft p-4">
      <div className="min-w-0">
        <div className="flex flex-wrap items-center gap-2">
          <FileCode2 className="size-4 shrink-0 text-graphite" />
          <p className="font-medium">{notebook.title}</p>
          <span
            className={cn(
              'rounded-full border px-2 py-0.5 text-[11px] font-medium uppercase tracking-wide',
              notebook.isMine ? 'border-ink text-ink' : 'border-line text-graphite',
            )}
          >
            {notebook.isMine ? 'yours' : 'public'}
          </span>
          {notebook.tags.map((tag) => (
            <span key={tag} className="text-[11px] text-graphite">
              #{tag}
            </span>
          ))}
        </div>
        {notebook.description && (
          <p className="mt-1 text-sm leading-relaxed text-graphite">{notebook.description}</p>
        )}
        <p className="mt-1.5 flex items-center gap-1.5 text-xs text-graphite">
          <BookOpen className="size-3.5" />
          {paramCount === 0
            ? 'no parameters'
            : `${paramCount} parameter${paramCount === 1 ? '' : 's'}`}
          {notebook.forkedFrom && <span>· forked from {notebook.forkedFrom}</span>}
          {notebook.author && !notebook.isMine && <span>· by {notebook.author}</span>}
        </p>
      </div>
      <div className="flex shrink-0 flex-wrap items-center gap-2">
        {onEdit && (
          <Button variant="secondary" size="sm" onClick={onEdit}>
            <Pencil className="size-4" /> Edit
          </Button>
        )}
        {onExport && (
          <Button
            variant="ghost"
            size="sm"
            onClick={onExport}
            aria-label={`Export ${notebook.title}`}
          >
            <Download className="size-4" />
          </Button>
        )}
        {onDelete && (
          <Button variant="danger" size="sm" onClick={onDelete}>
            <Trash2 className="size-4" /> Delete
          </Button>
        )}
        {onFork && (
          <Button variant="secondary" size="sm" onClick={onFork}>
            <Copy className="size-4" /> Fork
          </Button>
        )}
        <Button
          size="sm"
          disabled={!canRun}
          onClick={() => onRun(notebook)}
          title={canRun ? `Run on ${runtimeName}` : 'Connect a runtime first'}
        >
          <Play className="size-4" /> Run
        </Button>
      </div>
    </li>
  )
}
