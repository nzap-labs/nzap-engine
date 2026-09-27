import { useMemo, useState } from 'react'
import { Plus, Save, X } from 'lucide-react'
import { toast } from 'sonner'
import { useCreateNotebook, useUpdateNotebook } from '@/api/notebooks'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/dialog'
import { NOTEBOOK_PARAM_TYPES, type Notebook, type NotebookParam } from '@/types/notebook'

type EditorMode = 'create' | 'edit' | 'fork'

/**
 * Author a notebook: metadata, the Python source, and its declared parameter
 * schema. "Fork" opens this prefilled with a public notebook; saving always
 * creates a notebook in your own (local) collection — public ones are
 * read-only.
 */
export function NotebookEditorDialog({
  mode,
  notebook,
  onClose,
}: {
  mode: EditorMode
  notebook?: Notebook
  onClose: () => void
}) {
  const create = useCreateNotebook()
  const update = useUpdateNotebook()

  const initial = useMemo(() => initialDraft(mode, notebook), [mode, notebook])
  const [title, setTitle] = useState(initial.title)
  const [slug, setSlug] = useState(initial.slug)
  const [description, setDescription] = useState(initial.description)
  const [source, setSource] = useState(initial.source)
  const [params, setParams] = useState<NotebookParam[]>(initial.params)

  const slugTouched = slug !== initial.slug
  const busy = create.isPending || update.isPending
  const editing = mode === 'edit' && notebook

  function slugify(value: string): string {
    return typingSlug(value).replace(/-+$/g, '')
  }

  /** Like slugify, but keeps a trailing dash so `my-` can become `my-print`. */
  function typingSlug(value: string): string {
    return value
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, '-')
      .replace(/^-+/g, '')
      .slice(0, 63)
  }

  function save() {
    if (busy) return
    const cleanTitle = title.trim()
    if (!cleanTitle) return toast.error('A title is required.')
    if (!source.trim()) return toast.error('The notebook needs some Python source.')

    const draft = {
      slug: slugify(slug),
      title: cleanTitle,
      description: description.trim(),
      source,
      params: params.map(normaliseParam),
    }

    if (editing) {
      update.mutate(
        { id: notebook!.id, draft },
        {
          onSuccess: () => {
            toast.success('Notebook saved.')
            onClose()
          },
          onError: (error) => toast.error(error instanceof Error ? error.message : 'Save failed.'),
        },
      )
    } else {
      const forkedFrom =
        mode === 'fork' && notebook?.visibility === 'public' ? notebook.slug : notebook?.forkedFrom
      create.mutate(
        { ...draft, forkedFrom: mode === 'fork' ? (forkedFrom ?? null) : null },
        {
          onSuccess: () => {
            toast.success('Notebook saved to your collection.')
            onClose()
          },
          onError: (error) => toast.error(error instanceof Error ? error.message : 'Save failed.'),
        },
      )
    }
  }

  function updateParam(index: number, changes: Partial<NotebookParam>) {
    setParams((current) =>
      current.map((param, i) => {
        if (i !== index) return param
        const next = { ...param, ...changes }
        if (changes.type && changes.type !== param.type) {
          // A type switch resets the default so stale values cannot survive.
          next.default = changes.type === 'boolean' ? false : ''
        }
        return next
      }),
    )
  }

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="max-h-[90dvh] overflow-y-auto">
        <DialogTitle>
          {mode === 'create'
            ? 'New notebook'
            : mode === 'fork'
              ? `Fork “${notebook?.title}”`
              : `Edit “${notebook?.title}”`}
        </DialogTitle>
        <DialogDescription>
          {mode === 'fork'
            ? 'Creates a copy in your notebooks, stored on this computer. Tweak it before saving.'
            : 'Notebooks are private to you. Read the `params` dict in your code; NZAP injects it before the script runs.'}
        </DialogDescription>

        <div className="mt-4 grid gap-4">
          <label className="block">
            <span className="text-xs font-medium uppercase tracking-[0.14em] text-graphite">
              Title
            </span>
            <input
              value={title}
              onChange={(event) => {
                setTitle(event.target.value)
                if (!slugTouched) setSlug(slugify(event.target.value))
              }}
              className="mt-1.5 h-11 w-full rounded-2xl border border-ink bg-transparent px-4 text-sm outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ink"
            />
          </label>

          <label className="block">
            <span className="text-xs font-medium uppercase tracking-[0.14em] text-graphite">
              Slug{' '}
              <span className="normal-case tracking-normal">(unique handle, used in URLs)</span>
            </span>
            <input
              value={slug}
              onChange={(event) => setSlug(typingSlug(event.target.value))}
              className="mt-1.5 h-11 w-full rounded-2xl border border-ink bg-transparent px-4 font-mono text-sm outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ink"
            />
          </label>

          <label className="block">
            <span className="text-xs font-medium uppercase tracking-[0.14em] text-graphite">
              Description
            </span>
            <input
              value={description}
              onChange={(event) => setDescription(event.target.value)}
              className="mt-1.5 h-11 w-full rounded-2xl border border-ink bg-transparent px-4 text-sm outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ink"
            />
          </label>

          <label className="block">
            <span className="text-xs font-medium uppercase tracking-[0.14em] text-graphite">
              Source{' '}
              <span className="normal-case tracking-normal">
                (Python; a `params` dict is injected above it)
              </span>
            </span>
            <textarea
              value={source}
              onChange={(event) => setSource(event.target.value)}
              rows={10}
              spellCheck={false}
              className="mt-1.5 w-full resize-y rounded-2xl border border-ink bg-transparent p-4 font-mono text-sm outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ink"
            />
          </label>

          <div>
            <div className="flex items-center justify-between">
              <span className="text-xs font-medium uppercase tracking-[0.14em] text-graphite">
                Parameters
              </span>
              <Button
                variant="secondary"
                size="sm"
                onClick={() =>
                  setParams((current) => [
                    ...current,
                    {
                      key: `param_${current.length + 1}`,
                      label: `Parameter ${current.length + 1}`,
                      type: 'string',
                      default: '',
                      required: false,
                    },
                  ])
                }
              >
                <Plus className="size-4" /> Add
              </Button>
            </div>

            {params.length === 0 ? (
              <p className="mt-2 text-sm text-graphite">No parameters — the notebook runs as-is.</p>
            ) : (
              <ul className="mt-2 space-y-3">
                {params.map((param, index) => (
                  <li key={index} className="rounded-2xl border border-line bg-paper-soft p-3">
                    <div className="flex items-center justify-between gap-2">
                      <span className="truncate font-mono text-xs text-graphite">{param.key}</span>
                      <button
                        type="button"
                        aria-label={`Remove ${param.label}`}
                        onClick={() =>
                          setParams((current) => current.filter((_, i) => i !== index))
                        }
                        className="cursor-pointer rounded-lg p-1.5 text-graphite transition-colors hover:bg-paper hover:text-coral"
                      >
                        <X className="size-3.5" />
                      </button>
                    </div>
                    <div className="mt-2 grid gap-2 sm:grid-cols-2">
                      <label className="block">
                        <span className="text-[11px] text-graphite">Label</span>
                        <input
                          value={param.label}
                          onChange={(event) => updateParam(index, { label: event.target.value })}
                          className="mt-0.5 h-9 w-full rounded-xl border border-ink bg-transparent px-3 text-sm outline-none"
                        />
                      </label>
                      <label className="block">
                        <span className="text-[11px] text-graphite">Key (Python identifier)</span>
                        <input
                          value={param.key}
                          onChange={(event) => updateParam(index, { key: event.target.value })}
                          className="mt-0.5 h-9 w-full rounded-xl border border-ink bg-transparent px-3 font-mono text-sm outline-none"
                        />
                      </label>
                      <label className="block">
                        <span className="text-[11px] text-graphite">Type</span>
                        <select
                          value={param.type}
                          onChange={(event) =>
                            updateParam(index, {
                              type: event.target.value as NotebookParam['type'],
                            })
                          }
                          className="mt-0.5 h-9 w-full rounded-xl border border-ink bg-transparent px-2 text-sm outline-none"
                        >
                          {NOTEBOOK_PARAM_TYPES.map((type) => (
                            <option key={type} value={type}>
                              {type}
                            </option>
                          ))}
                        </select>
                      </label>
                      <label className="block">
                        <span className="text-[11px] text-graphite">Default</span>
                        <input
                          value={
                            param.default === null || param.default === undefined
                              ? ''
                              : String(param.default)
                          }
                          onChange={(event) =>
                            updateParam(index, {
                              default:
                                param.type === 'integer' || param.type === 'number'
                                  ? event.target.value === ''
                                    ? ''
                                    : Number(event.target.value)
                                  : event.target.value,
                            })
                          }
                          disabled={param.type === 'boolean'}
                          className="mt-0.5 h-9 w-full rounded-xl border border-ink bg-transparent px-3 text-sm outline-none disabled:opacity-40"
                        />
                      </label>
                      {param.type === 'select' && (
                        <label className="block sm:col-span-2">
                          <span className="text-[11px] text-graphite">
                            Options (comma-separated)
                          </span>
                          <input
                            value={(param.options ?? []).join(', ')}
                            onChange={(event) =>
                              updateParam(index, {
                                options: event.target.value
                                  .split(',')
                                  .map((option) => option.trim())
                                  .filter(Boolean),
                              })
                            }
                            className="mt-0.5 h-9 w-full rounded-xl border border-ink bg-transparent px-3 text-sm outline-none"
                          />
                        </label>
                      )}
                      <label className="flex items-center gap-2 text-sm">
                        <input
                          type="checkbox"
                          checked={Boolean(param.required)}
                          onChange={(event) =>
                            updateParam(index, { required: event.target.checked })
                          }
                          className="size-4 accent-[var(--color-ink)]"
                        />
                        Required
                      </label>
                    </div>
                  </li>
                ))}
              </ul>
            )}
          </div>
        </div>

        <div className="mt-5 flex justify-end gap-2">
          <DialogClose asChild>
            <Button variant="ghost" size="sm" disabled={busy}>
              Cancel
            </Button>
          </DialogClose>
          <Button size="sm" disabled={busy || !title.trim() || !source.trim()} onClick={save}>
            {busy ? <Plus className="size-4 animate-pulse" /> : <Save className="size-4" />}
            {editing ? 'Save changes' : 'Create notebook'}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  )
}

function initialDraft(
  mode: EditorMode,
  notebook?: Notebook,
): {
  title: string
  slug: string
  description: string
  source: string
  params: NotebookParam[]
} {
  if (mode === 'create' || !notebook) {
    return {
      title: '',
      slug: '',
      description: '',
      source: 'print("hello from my notebook")\n',
      params: [],
    }
  }
  const fork = mode === 'fork'
  return {
    title: fork ? `${notebook.title} (fork)` : notebook.title,
    slug: fork ? '' : notebook.slug,
    description: notebook.description,
    source: notebook.source ?? '',
    params: notebook.params.map((param) => ({ ...param })),
  }
}

/** Coerce editor state into what the backend schema expects. */
function normaliseParam(param: NotebookParam): NotebookParam {
  const cleaned: NotebookParam = {
    key: param.key.trim(),
    label: param.label.trim() || param.key.trim(),
    type: param.type,
    required: Boolean(param.required),
  }
  if (param.type === 'boolean') {
    cleaned.default = param.default === true
  } else if (param.default !== '' && param.default !== undefined && param.default !== null) {
    cleaned.default =
      param.type === 'integer' || param.type === 'number'
        ? Number(param.default)
        : String(param.default)
  } else {
    cleaned.default = null
  }
  if (param.type === 'select') cleaned.options = param.options ?? []
  if (param.description?.trim()) cleaned.description = param.description.trim()
  return cleaned
}
