import { useRef, useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { Download, FileUp, Link2, Loader2, Play, Square } from 'lucide-react'
import { toast } from 'sonner'
import { saveTextFile, streamRunFile, useImportNotebook, useSessionAction } from '@/api/colab'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/cn'
import type { ColabRunFileEvent } from '@/types/colab'
import { Block, renderMimeBundle, type OutputBlock } from './output-view'

const FIELD =
  'h-10 w-full rounded-2xl border border-ink bg-transparent px-4 text-sm outline-none placeholder:text-graphite/60 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ink'

interface CellRun {
  index: number
  status: 'running' | 'ok' | 'error'
  blocks: OutputBlock[]
}

let nextId = 1

/**
 * `colab exec -f FILE [--env K=V]` in the browser: pick a local .py / .ipynb
 * (or import one from a Colab, Drive or GitHub link), run it on the active
 * runtime cell by cell, and download the executed notebook.
 */
export function RunFilePanel({ sessionName }: { sessionName: string | null }) {
  const queryClient = useQueryClient()
  const importNotebook = useImportNotebook()
  const interrupt = useSessionAction()
  const [file, setFile] = useState<{ filename: string; content: string } | null>(null)
  const [url, setUrl] = useState('')
  const [env, setEnv] = useState('')
  const [stopOnError, setStopOnError] = useState(false)
  const [running, setRunning] = useState(false)
  const [cells, setCells] = useState<CellRun[]>([])
  const [total, setTotal] = useState(0)
  const [result, setResult] = useState<{ status: string; filename?: string; notebook?: unknown }>()
  const abortRef = useRef<AbortController | null>(null)
  const picker = useRef<HTMLInputElement>(null)

  if (!sessionName) {
    return (
      <section className="rounded-[24px] border border-line bg-paper p-8 text-center">
        <FileUp className="mx-auto size-6 text-graphite" />
        <p className="mt-3 text-sm text-graphite">Select a runtime to run a file on it.</p>
      </section>
    )
  }

  function updateCurrent(update: (cell: CellRun) => CellRun) {
    setCells((current) =>
      current.length ? [...current.slice(0, -1), update(current[current.length - 1]!)] : current,
    )
  }

  function push(block: Omit<OutputBlock, 'id'>) {
    updateCurrent((cell) => ({ ...cell, blocks: [...cell.blocks, { ...block, id: nextId++ }] }))
  }

  function onEvent(event: ColabRunFileEvent) {
    switch (event.type) {
      case 'cell':
        setTotal(event.total)
        if (event.state === 'started') {
          setCells((current) => [...current, { index: event.index, status: 'running', blocks: [] }])
        } else {
          updateCurrent((cell) => ({ ...cell, status: event.status === 'ok' ? 'ok' : 'error' }))
        }
        break
      case 'stream':
        push({ kind: 'stream', streamName: event.name ?? 'stdout', text: event.text ?? '' })
        break
      case 'result':
      case 'display':
      case 'update_display':
        push({ kind: 'result', ...renderMimeBundle(event.data) })
        break
      case 'error':
        push({
          kind: 'error',
          text:
            (event.traceback ?? []).join('\n') ||
            `${event.ename ?? 'Error'}: ${event.evalue ?? ''}`,
        })
        break
      case 'clear_output':
        updateCurrent((cell) => ({ ...cell, blocks: [] }))
        break
      case 'colab_request':
      case 'drive_auth_required':
        push({ kind: 'status', text: event.message ?? '' })
        break
      case 'run_complete':
        setResult({ status: event.status, filename: event.filename, notebook: event.notebook })
        break
      default:
        break
    }
  }

  async function pick(picked: File | undefined) {
    if (!picked) return
    if (!/\.(py|ipynb)$/i.test(picked.name)) {
      toast.error('Pick a .py or .ipynb file.')
      return
    }
    if (picked.size > 20 * 1024 * 1024) {
      toast.error('That file is larger than 20 MB.')
      return
    }
    setFile({ filename: picked.name, content: await picked.text() })
    setResult(undefined)
  }

  function fetchUrl() {
    if (!url.trim()) return
    importNotebook.mutate(url.trim(), {
      onSuccess: (imported) => {
        setFile({ filename: imported.filename, content: imported.content })
        setResult(undefined)
        toast.success(`Imported ${imported.filename}.`)
      },
      onError: (error) => toast.error(error instanceof Error ? error.message : 'Import failed.'),
    })
  }

  async function run() {
    if (!file || running || !sessionName) return
    const envList = env
      .split('\n')
      .map((line) => line.trim())
      .filter(Boolean)
    setRunning(true)
    setCells([])
    setTotal(0)
    setResult(undefined)
    const controller = new AbortController()
    abortRef.current = controller
    try {
      await streamRunFile(
        sessionName,
        { ...file, env: envList, stopOnError },
        { onEvent, signal: controller.signal },
      )
    } catch (error) {
      if (!controller.signal.aborted) {
        toast.error(error instanceof Error ? error.message : 'Run failed.')
      }
    } finally {
      abortRef.current = null
      setRunning(false)
      void queryClient.invalidateQueries({ queryKey: ['colab', 'history', sessionName] })
    }
  }

  function stop() {
    abortRef.current?.abort()
    interrupt.mutate({ name: sessionName!, action: 'interrupt' })
  }

  const finished = cells.filter((cell) => cell.status !== 'running').length

  return (
    <section aria-label="Run a file" className="rounded-[24px] border border-ink bg-paper p-6">
      <p className="font-medium">Run a file</p>
      <p className="mt-1 text-sm text-graphite">
        Runs a local script or notebook on <span className="font-medium">{sessionName}</span>{' '}
        without uploading it first. Notebooks come back with their outputs.
      </p>

      <div className="mt-4 space-y-3">
        <div className="flex flex-wrap gap-2">
          <input
            ref={picker}
            type="file"
            accept=".py,.ipynb"
            className="hidden"
            onChange={(event) => void pick(event.target.files?.[0])}
          />
          <Button variant="secondary" size="sm" onClick={() => picker.current?.click()}>
            <FileUp className="size-4" /> Choose .py / .ipynb
          </Button>
          <div className="flex min-w-0 flex-1 gap-2">
            <input
              value={url}
              onChange={(event) => setUrl(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === 'Enter') fetchUrl()
              }}
              placeholder="or paste a Colab, Drive or GitHub notebook link"
              aria-label="Notebook link"
              className={cn(FIELD, 'min-w-0 flex-1')}
            />
            <Button
              variant="secondary"
              size="sm"
              onClick={fetchUrl}
              disabled={importNotebook.isPending || !url.trim()}
            >
              {importNotebook.isPending ? (
                <Loader2 className="size-4 animate-spin" />
              ) : (
                <Link2 className="size-4" />
              )}
              Import
            </Button>
          </div>
        </div>

        {file && (
          <p className="text-xs text-graphite">
            Ready: <span className="font-mono text-ink">{file.filename}</span> (
            {(file.content.length / 1024).toFixed(1)} KB)
          </p>
        )}

        <label className="block">
          <span className="text-xs font-medium uppercase tracking-[0.14em] text-graphite">
            Environment (KEY=VALUE per line)
          </span>
          <textarea
            value={env}
            onChange={(event) => setEnv(event.target.value)}
            rows={2}
            spellCheck={false}
            placeholder={'EPOCHS=3\nWANDB_MODE=offline'}
            className="mt-1.5 w-full resize-y rounded-2xl border border-ink bg-transparent p-3 font-mono text-xs outline-none placeholder:text-graphite/60 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ink"
          />
        </label>

        <div className="flex flex-wrap items-center gap-3">
          <Button size="sm" onClick={() => void run()} disabled={!file || running}>
            {running ? <Loader2 className="size-4 animate-spin" /> : <Play className="size-4" />}
            {running ? `Running ${finished}/${total || '…'}` : 'Run'}
          </Button>
          <Button variant="secondary" size="sm" onClick={stop} disabled={!running}>
            <Square className="size-4" /> Stop
          </Button>
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={stopOnError}
              onChange={(event) => setStopOnError(event.target.checked)}
              className="size-4 accent-[var(--color-ink)]"
            />
            Stop at the first failing cell
          </label>
          {result?.notebook !== undefined && result.filename && (
            <Button
              variant="secondary"
              size="sm"
              onClick={() =>
                saveTextFile(result.filename!, JSON.stringify(result.notebook, null, 1))
                  .then((path) => path && toast.success(`Saved to ${path}.`))
                  .catch((error: unknown) =>
                    toast.error(error instanceof Error ? error.message : 'Could not save.'),
                  )
              }
            >
              <Download className="size-4" /> {result.filename}
            </Button>
          )}
        </div>
      </div>

      {cells.length > 0 && (
        <ol className="mt-4 max-h-[28rem] space-y-3 overflow-y-auto scrollbar-thin">
          {cells.map((cell) => (
            <li key={cell.index} className="rounded-2xl border border-line bg-paper-soft p-3">
              <p className="flex items-center justify-between text-xs text-graphite">
                <span>
                  Cell {cell.index + 1}
                  {total ? ` of ${total}` : ''}
                </span>
                <span
                  className={cn(
                    'font-medium',
                    cell.status === 'error' && 'text-coral',
                    cell.status === 'ok' && 'text-ink',
                  )}
                >
                  {cell.status === 'running' ? 'running…' : cell.status}
                </span>
              </p>
              {cell.blocks.length > 0 && (
                <div className="mt-2 font-mono text-xs leading-relaxed">
                  {cell.blocks.map((block) => (
                    <Block key={block.id} block={block} />
                  ))}
                </div>
              )}
            </li>
          ))}
        </ol>
      )}

      {result && (
        <p className={cn('mt-3 text-sm', result.status === 'ok' ? 'text-graphite' : 'text-coral')}>
          {result.status === 'ok' ? '✓ Every cell finished.' : '✗ Some cells failed.'}
        </p>
      )}
    </section>
  )
}
