import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import {
  ChevronRight,
  Download,
  File as FileIcon,
  FileCode2,
  Folder,
  FilePlus2,
  FolderPlus,
  Pencil,
  Save,
  Trash2,
  Upload,
} from 'lucide-react'
import { toast } from 'sonner'
import {
  colabFileContentQuery,
  colabFilesQuery,
  useDownloadFile,
  useFileCommand,
  useRenameFile,
  useSaveFile,
  useUploadFile,
} from '@/api/colab'
import { Button } from '@/components/ui/button'
import { useDialogs } from '@/components/dialogs'
import { cn } from '@/lib/cn'
import type { ColabFileEntry } from '@/types/colab'

const TEXT_EXTENSIONS = [
  '.py',
  '.txt',
  '.md',
  '.json',
  '.csv',
  '.tsv',
  '.yaml',
  '.yml',
  '.toml',
  '.ini',
  '.cfg',
  '.sh',
  '.bash',
  '.js',
  '.ts',
  '.html',
  '.css',
  '.sql',
  '.log',
  '.ipynb',
  '.r',
  '.java',
  '.c',
  '.cpp',
  '.h',
]

/** File manager for the active runtime, backed by the VM's Jupyter contents API. */
export function FilesPanel({ sessionName }: { sessionName: string | null }) {
  // Colab's Jupyter root is `/`; start where Colab's own file browser does.
  const [path, setPath] = useState('content')
  const [editing, setEditing] = useState<string | null>(null)
  // null = untouched: the editor shows the loaded content until the first edit.
  const [draft, setDraft] = useState<string | null>(null)

  const listing = useQuery(colabFilesQuery(sessionName, path))
  const content = useQuery(colabFileContentQuery(sessionName, editing))
  const save = useSaveFile()
  const command = useFileCommand()
  const upload = useUploadFile()
  const download = useDownloadFile()
  const rename = useRenameFile()
  const dialogs = useDialogs()
  const [dragging, setDragging] = useState(false)

  if (!sessionName) {
    return (
      <section className="rounded-[24px] border border-line bg-paper p-8 text-center">
        <Folder className="mx-auto size-6 text-graphite" />
        <p className="mt-3 text-sm text-graphite">Select a runtime to browse its files.</p>
      </section>
    )
  }

  const entries = listing.data?.entries ?? []

  function openEntry(entry: ColabFileEntry) {
    if (entry.type === 'directory') {
      navigate(entry.path)
      return
    }
    if (isTextFile(entry.name)) {
      setEditing(entry.path)
      setDraft(null)
      return
    }
    downloadEntry(entry.path)
  }

  function downloadEntry(target: string) {
    download.mutate(
      { name: sessionName!, path: target },
      {
        onSuccess: (saved) => saved && toast.success(`Saved to ${saved}.`),
        onError: () => toast.error('Could not download that file.'),
      },
    )
  }

  function navigate(next: string) {
    setPath(next)
    setEditing(null)
    setDraft(null)
  }

  function uploadAll(files: FileList | File[] | null | undefined) {
    for (const file of Array.from(files ?? [])) onUploadPicked(file)
  }

  function onUploadPicked(file: File | undefined) {
    if (!file) return
    upload.mutate(
      { name: sessionName!, path: joinPath(path, file.name), file },
      {
        onSuccess: () => toast.success(`Uploaded ${file.name}.`),
        onError: (error) => toast.error(error instanceof Error ? error.message : 'Upload failed.'),
      },
    )
  }

  return (
    <section
      aria-label="Files"
      className={cn(
        'rounded-[24px] border bg-paper p-6 transition-colors',
        dragging ? 'border-dashed border-ink bg-paper-soft' : 'border-ink',
      )}
      onDragOver={(event) => {
        event.preventDefault()
        setDragging(true)
      }}
      onDragLeave={() => setDragging(false)}
      onDrop={(event) => {
        event.preventDefault()
        setDragging(false)
        uploadAll(event.dataTransfer.files)
      }}
    >
      <div className="flex flex-wrap items-center justify-between gap-3">
        <Breadcrumb path={listing.data?.path ?? path} onNavigate={navigate} />
        <div className="flex gap-2">
          <label className="inline-flex h-9 cursor-pointer items-center gap-2 rounded-3xl border border-ink px-4 text-sm font-medium transition-colors hover:bg-paper-soft">
            <Upload className="size-4" /> Upload
            <input
              type="file"
              multiple
              className="hidden"
              onChange={(event) => {
                uploadAll(event.target.files)
                event.target.value = ''
              }}
            />
          </label>
          <Button
            variant="secondary"
            size="sm"
            disabled={command.isPending}
            onClick={async () => {
              const name = (
                await dialogs.prompt({ title: 'New folder', placeholder: 'data' })
              )?.trim()
              if (!name) return
              command.mutate(
                { name: sessionName!, command: 'mkdir', path: joinPath(path, name) },
                {
                  onSuccess: () => toast.success('Folder created.'),
                  onError: () => toast.error('Could not create the folder.'),
                },
              )
            }}
          >
            <FolderPlus className="size-4" /> New folder
          </Button>
          <Button
            variant="secondary"
            size="sm"
            disabled={save.isPending}
            onClick={async () => {
              const name = (
                await dialogs.prompt({ title: 'New file', defaultValue: 'untitled.py' })
              )?.trim()
              if (!name) return
              const target = joinPath(path, name)
              save.mutate(
                { name: sessionName!, path: target, content: '' },
                {
                  onSuccess: () => {
                    toast.success('File created.')
                    if (isTextFile(name)) {
                      setEditing(target)
                      setDraft('')
                    }
                  },
                  onError: () => toast.error('Could not create the file.'),
                },
              )
            }}
          >
            <FilePlus2 className="size-4" /> New file
          </Button>
        </div>
      </div>

      {listing.isPending ? (
        <div className="mt-4 space-y-2">
          <div className="h-8 animate-pulse rounded-lg bg-paper-soft" />
          <div className="h-8 animate-pulse rounded-lg bg-paper-soft" />
        </div>
      ) : entries.length === 0 ? (
        <p className="mt-4 text-sm text-graphite">This folder is empty.</p>
      ) : (
        <ul className="mt-4 divide-y divide-line">
          {entries.map((entry) => (
            <li key={entry.path} className="flex items-center gap-3 py-2.5">
              <button
                type="button"
                onClick={() => openEntry(entry)}
                className="flex min-w-0 flex-1 cursor-pointer items-center gap-3 text-left"
              >
                {entry.type === 'directory' ? (
                  <Folder className="size-4 shrink-0 text-graphite" />
                ) : isTextFile(entry.name) ? (
                  <FileCode2 className="size-4 shrink-0 text-graphite" />
                ) : (
                  <FileIcon className="size-4 shrink-0 text-graphite" />
                )}
                <span className="min-w-0 flex-1 truncate text-sm">{entry.name}</span>
                {entry.size != null && entry.type !== 'directory' && (
                  <span className="shrink-0 text-xs text-graphite">{formatSize(entry.size)}</span>
                )}
              </button>
              <div className="flex shrink-0 gap-1">
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label={`Download ${entry.name}`}
                  onClick={() => downloadEntry(entry.path)}
                >
                  <Download className="size-4" />
                </Button>
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label={`Rename ${entry.name}`}
                  disabled={rename.isPending}
                  onClick={async () => {
                    const next = (
                      await dialogs.prompt({
                        title: `Rename ${entry.name}`,
                        defaultValue: entry.name,
                      })
                    )?.trim()
                    if (!next || next === entry.name) return
                    const parent = entry.path.split('/').slice(0, -1).join('/')
                    const newPath = next.includes('/') ? next : joinPath(parent, next)
                    rename.mutate(
                      { name: sessionName!, path: entry.path, newPath },
                      {
                        onSuccess: () => {
                          toast.success(`Renamed to ${next}.`)
                          if (editing === entry.path) setEditing(null)
                        },
                        onError: (error) =>
                          toast.error(error instanceof Error ? error.message : 'Rename failed.'),
                      },
                    )
                  }}
                >
                  <Pencil className="size-4" />
                </Button>
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label={`Delete ${entry.name}`}
                  onClick={async () => {
                    const confirmed = await dialogs.confirm({
                      title: `Delete ${entry.name}?`,
                      description: `${entry.path} is removed from the runtime.`,
                      confirmLabel: 'Delete',
                      danger: true,
                    })
                    if (!confirmed) return
                    command.mutate(
                      { name: sessionName!, command: 'delete', path: entry.path },
                      {
                        onSuccess: () => toast.success(`Deleted ${entry.name}.`),
                        onError: () => toast.error('Could not delete that entry.'),
                      },
                    )
                  }}
                >
                  <Trash2 className="size-4" />
                </Button>
              </div>
            </li>
          ))}
        </ul>
      )}

      {editing && (
        <div className="mt-5 border-t border-line pt-5">
          <div className="flex items-center justify-between gap-3">
            <p className="truncate font-mono text-xs text-graphite">{editing}</p>
            <div className="flex shrink-0 gap-2">
              <Button
                size="sm"
                disabled={save.isPending || (content.isPending && draft === null)}
                onClick={() => {
                  if (content.isPending && draft === null) {
                    toast.error('The file is still loading.')
                    return
                  }
                  const body = draft ?? fileText(content.data?.file?.content)
                  save.mutate(
                    { name: sessionName!, path: editing, content: body },
                    {
                      onSuccess: () => toast.success('File saved.'),
                      onError: (error) =>
                        toast.error(error instanceof Error ? error.message : 'Could not save.'),
                    },
                  )
                }}
              >
                <Save className="size-4" /> Save
              </Button>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => {
                  setEditing(null)
                  setDraft(null)
                }}
              >
                Close
              </Button>
            </div>
          </div>
          <textarea
            value={draft ?? fileText(content.data?.file?.content)}
            onChange={(event) => setDraft(event.target.value)}
            rows={14}
            spellCheck={false}
            className="mt-3 w-full resize-y rounded-2xl border border-ink bg-transparent p-4 font-mono text-sm outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ink"
          />
          {content.isPending && <p className="mt-2 text-xs text-graphite">Loading…</p>}
        </div>
      )}
    </section>
  )
}

function Breadcrumb({ path, onNavigate }: { path: string; onNavigate: (path: string) => void }) {
  const parts = path.split('/').filter(Boolean)
  return (
    <nav aria-label="File path" className="flex min-w-0 items-center gap-1 text-sm">
      <button
        type="button"
        onClick={() => onNavigate('')}
        aria-label="Root folder"
        className="cursor-pointer rounded px-1 font-mono text-graphite transition-colors hover:text-ink"
      >
        /
      </button>
      {parts.map((part, index) => (
        <span key={`${part}-${index}`} className="flex items-center gap-1">
          <ChevronRight className="size-3.5 text-graphite" />
          <button
            type="button"
            onClick={() => onNavigate(parts.slice(0, index + 1).join('/'))}
            className={cn(
              'cursor-pointer rounded px-1 transition-colors hover:text-ink',
              index === parts.length - 1 ? 'font-medium text-ink' : 'text-graphite',
            )}
          >
            {part}
          </button>
        </span>
      ))}
    </nav>
  )
}

/** Text of a contents model (notebooks arrive as parsed JSON). */
function fileText(content: unknown): string {
  if (typeof content === 'string') return content
  if (content === null || content === undefined) return ''
  return JSON.stringify(content, null, 1)
}

function isTextFile(name: string): boolean {
  const lower = name.toLowerCase()
  return TEXT_EXTENSIONS.some((extension) => lower.endsWith(extension))
}

function joinPath(path: string, name: string): string {
  return path ? `${path.replace(/\/$/, '')}/${name}` : name
}

function formatSize(bytes: number): string {
  const units = ['B', 'KB', 'MB', 'GB']
  let value = bytes
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit += 1
  }
  return `${value.toFixed(value >= 10 || unit === 0 ? 0 : 1)} ${units[unit]}`
}
