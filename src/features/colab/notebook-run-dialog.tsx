import { useEffect, useRef, useState } from 'react'
import { Loader2, Play, Square } from 'lucide-react'
import { streamNotebookRun } from '@/api/notebooks'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/dialog'
import type { ColabExecuteEvent } from '@/types/colab'
import type { Notebook, NotebookParamValues } from '@/types/notebook'

interface RunSession {
  name: string
  accelerator: string
}

interface OutputBlock {
  id: number
  kind: 'stream' | 'result' | 'error' | 'status'
  text: string
}

let nextId = 1

/**
 * Run form for a notebook: collect its declared parameters, pick a runtime,
 * and stream the execution's output right into the dialog.
 */
export function NotebookRunDialog({
  notebook,
  sessions,
  defaultSession,
  onClose,
}: {
  notebook: Notebook
  sessions: RunSession[]
  defaultSession: string | null
  onClose: () => void
}) {
  const [values, setValues] = useState<NotebookParamValues>(() => defaults(notebook))
  const [session, setSession] = useState(defaultSession ?? sessions[0]?.name ?? '')
  const [running, setRunning] = useState(false)
  const [blocks, setBlocks] = useState<OutputBlock[]>([])
  const abortRef = useRef<AbortController | null>(null)
  const outputRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    outputRef.current?.scrollTo({ top: outputRef.current.scrollHeight })
  }, [blocks])

  function append(block: Omit<OutputBlock, 'id'>) {
    setBlocks((current) => [...current, { ...block, id: nextId++ }])
  }

  function handleEvent(event: ColabExecuteEvent) {
    switch (event.type) {
      case 'stream':
        append({ kind: 'stream', text: event.text ?? '' })
        break
      case 'result':
      case 'display':
      case 'update_display': {
        const data = event.data
        const text = data
          ? ((Array.isArray(data['text/plain'])
              ? data['text/plain'].join('')
              : data['text/plain']) ?? `[${Object.keys(data).join(', ') || 'empty'} output]`)
          : ''
        append({ kind: 'result', text })
        break
      }
      case 'error':
        append({
          kind: 'error',
          text:
            (event.traceback ?? []).join('\n') ||
            `${event.ename ?? 'Error'}: ${event.evalue ?? ''}`,
        })
        break
      case 'clear_output':
        setBlocks([])
        break
      case 'colab_request':
        append({ kind: 'status', text: event.message ?? '' })
        break
      case 'execute_reply':
        append({
          kind: 'status',
          text:
            event.status === 'ok'
              ? `✓ finished · ${new Date().toLocaleTimeString()}`
              : '✗ finished with errors',
        })
        break
      default:
        break
    }
  }

  async function run() {
    if (!session || running) return
    setRunning(true)
    setBlocks([])
    const controller = new AbortController()
    abortRef.current = controller
    try {
      await streamNotebookRun(
        notebook.id,
        { session, params: values },
        {
          onEvent: handleEvent,
          signal: controller.signal,
        },
      )
    } catch (error) {
      if (!controller.signal.aborted) {
        append({ kind: 'error', text: error instanceof Error ? error.message : 'Run failed.' })
      }
    } finally {
      setRunning(false)
      abortRef.current = null
    }
  }

  function close() {
    abortRef.current?.abort()
    onClose()
  }

  const missingRequired = notebook.params.some(
    (param) => param.required && (values[param.key] === undefined || values[param.key] === ''),
  )

  return (
    <Dialog open onOpenChange={(open) => !open && close()}>
      <DialogContent className="max-h-[85dvh] overflow-y-auto">
        <DialogTitle>{notebook.title}</DialogTitle>
        <DialogDescription>
          {notebook.description || `Runs on ${session || 'a runtime'}.`}
        </DialogDescription>

        {notebook.params.length > 0 && (
          <div className="mt-4 space-y-4">
            {notebook.params.map((param) => (
              <label key={param.key} className="block">
                <span className="text-xs font-medium uppercase tracking-[0.14em] text-graphite">
                  {param.label}
                  {param.required && ' *'}
                </span>
                <ParamInput
                  param={param}
                  value={values[param.key]}
                  onChange={(value) => setValues((current) => ({ ...current, [param.key]: value }))}
                />
                {param.description && (
                  <span className="mt-1 block text-xs text-graphite">{param.description}</span>
                )}
              </label>
            ))}
          </div>
        )}

        <label className="mt-4 block">
          <span className="text-xs font-medium uppercase tracking-[0.14em] text-graphite">
            Runtime
          </span>
          <select
            value={session}
            onChange={(event) => setSession(event.target.value)}
            className="mt-1.5 h-11 w-full rounded-2xl border border-ink bg-transparent px-3 text-sm outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ink"
          >
            {sessions.map((option) => (
              <option key={option.name} value={option.name}>
                {option.name} · {option.accelerator}
              </option>
            ))}
          </select>
        </label>

        <div className="mt-5 flex justify-end gap-2">
          <DialogClose asChild>
            <Button variant="ghost" size="sm" disabled={running}>
              Close
            </Button>
          </DialogClose>
          {running ? (
            <Button variant="secondary" size="sm" onClick={() => abortRef.current?.abort()}>
              <Square className="size-4" /> Stop
            </Button>
          ) : (
            <Button size="sm" disabled={!session || missingRequired} onClick={run}>
              <Play className="size-4" /> Run
            </Button>
          )}
        </div>

        {(running || blocks.length > 0) && (
          <div
            ref={outputRef}
            className="mt-4 max-h-64 overflow-y-auto rounded-2xl border border-line bg-paper-soft p-4 font-mono text-xs leading-relaxed scrollbar-thin"
          >
            {running && blocks.length === 0 && (
              <p className="flex items-center gap-2 text-graphite">
                <Loader2 className="size-3.5 animate-spin" /> Waiting for the runtime…
              </p>
            )}
            {blocks.map((block) => (
              <pre
                key={block.id}
                className={
                  block.kind === 'error'
                    ? 'whitespace-pre-wrap break-words text-coral'
                    : block.kind === 'status'
                      ? 'whitespace-pre-wrap break-words text-graphite'
                      : 'whitespace-pre-wrap break-words'
                }
              >
                {block.text}
              </pre>
            ))}
          </div>
        )}
      </DialogContent>
    </Dialog>
  )
}

function defaults(notebook: Notebook): NotebookParamValues {
  const values: NotebookParamValues = {}
  for (const param of notebook.params) {
    if (param.default !== null && param.default !== undefined) {
      values[param.key] = param.default
    }
  }
  return values
}

function ParamInput({
  param,
  value,
  onChange,
}: {
  param: Notebook['params'][number]
  value: NotebookParamValues[string] | undefined
  onChange: (value: NotebookParamValues[string]) => void
}) {
  const classes =
    'mt-1.5 w-full rounded-2xl border border-ink bg-transparent px-4 text-sm outline-none placeholder:text-graphite/60 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ink'

  if (param.type === 'text') {
    return (
      <textarea
        value={String(value ?? '')}
        onChange={(event) => onChange(event.target.value)}
        rows={3}
        className={`${classes} resize-y py-3 font-mono`}
      />
    )
  }
  if (param.type === 'select') {
    return (
      <select
        value={String(value ?? '')}
        onChange={(event) => onChange(event.target.value)}
        className={`${classes} h-11`}
      >
        {(param.options ?? []).map((option) => (
          <option key={option} value={option}>
            {option}
          </option>
        ))}
      </select>
    )
  }
  if (param.type === 'boolean') {
    return (
      <label className="mt-1.5 flex items-center gap-2.5 text-sm">
        <input
          type="checkbox"
          checked={value === true}
          onChange={(event) => onChange(event.target.checked)}
          className="size-4 accent-[var(--color-ink)]"
        />
        {param.description || param.label}
      </label>
    )
  }
  return (
    <input
      type={param.type === 'integer' || param.type === 'number' ? 'number' : 'text'}
      step={param.type === 'integer' ? 1 : 'any'}
      value={String(value ?? '')}
      onChange={(event) =>
        onChange(
          param.type === 'integer' || param.type === 'number'
            ? event.target.value === ''
              ? ''
              : Number(event.target.value)
            : event.target.value,
        )
      }
      className={`${classes} h-11`}
    />
  )
}
