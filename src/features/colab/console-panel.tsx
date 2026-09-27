import { useEffect, useRef, useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { Play, Square, Terminal } from 'lucide-react'
import { toast } from 'sonner'
import { useSessionAction, useSendStdin, streamExecute } from '@/api/colab'
import { Button } from '@/components/ui/button'
import { ExternalLink } from '@/components/external-link'
import type { ColabExecuteEvent } from '@/types/colab'
import { Block, renderMimeBundle, type OutputBlock } from './output-view'

let nextId = 1

/**
 * Notebook-style console for the active runtime: run a cell, watch outputs
 * stream in, answer input prompts, and retry the Drive mount when the VM asks
 * for credentials.
 */
export function ConsolePanel({ sessionName }: { sessionName: string | null }) {
  const [code, setCode] = useState('print("hello from NZAP")')
  const [blocks, setBlocks] = useState<OutputBlock[]>([])
  const [cellLabel, setCellLabel] = useState('In [ ]')
  const [running, setRunning] = useState(false)
  const [stdinPrompt, setStdinPrompt] = useState<string | null>(null)
  const [stdinValue, setStdinValue] = useState('')
  const [driveRequest, setDriveRequest] = useState<{ message: string; uri?: string } | null>(null)
  const abortRef = useRef<AbortController | null>(null)
  const outputRef = useRef<HTMLDivElement>(null)
  const sendStdin = useSendStdin()
  const driveAuthorize = useSessionAction()
  const queryClient = useQueryClient()

  // Keep the transcript scrolled to the newest output.
  useEffect(() => {
    outputRef.current?.scrollTo({ top: outputRef.current.scrollHeight })
  }, [blocks])

  function append(block: Omit<OutputBlock, 'id'>) {
    setBlocks((current) => [...current, { ...block, id: nextId++ }])
  }

  function handleEvent(event: ColabExecuteEvent) {
    switch (event.type) {
      case 'stream':
        append({ kind: 'stream', streamName: event.name ?? 'stdout', text: event.text ?? '' })
        break
      case 'result': {
        const rendered = renderMimeBundle(event.data)
        if (typeof event.execution_count === 'number') {
          setCellLabel(`Out [${event.execution_count}]`)
        }
        append({ kind: 'result', ...rendered })
        break
      }
      case 'display':
      case 'update_display':
        append({ kind: 'result', ...renderMimeBundle(event.data) })
        break
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
      case 'input':
        if (typeof event.execution_count === 'number') setCellLabel(`In [${event.execution_count}]`)
        break
      case 'input_request':
        setStdinPrompt(event.prompt ?? 'Input requested')
        break
      case 'colab_request':
        append({ kind: 'status', text: event.message ?? '' })
        break
      case 'drive_auth_required':
        setDriveRequest({
          message: event.message ?? 'The runtime needs Google Drive access.',
          uri: event.uri,
        })
        append({ kind: 'status', text: event.message ?? '' })
        break
      case 'execute_reply':
        void queryClient.invalidateQueries({ queryKey: ['colab', 'history', sessionName] })
        setRunning(false)
        setStdinPrompt(null)
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
    if (!sessionName || running || !code.trim()) return
    setRunning(true)
    setBlocks([])
    setCellLabel('In [*]')
    setDriveRequest(null)
    const controller = new AbortController()
    abortRef.current = controller
    try {
      await streamExecute(sessionName, code, { onEvent: handleEvent, signal: controller.signal })
    } catch (error) {
      if (!controller.signal.aborted) {
        append({ kind: 'error', text: error instanceof Error ? error.message : 'Run failed.' })
      }
      setRunning(false)
    } finally {
      abortRef.current = null
    }
  }

  function interrupt() {
    if (!sessionName) return
    abortRef.current?.abort()
    driveAuthorize.mutate(
      { name: sessionName, action: 'interrupt' },
      {
        onSuccess: () => toast('Interrupt sent.'),
        onError: (error) =>
          toast.error(error instanceof Error ? error.message : 'Interrupt failed.'),
      },
    )
    setRunning(false)
  }

  function answerStdin() {
    if (!sessionName) return
    const value = stdinValue
    setStdinPrompt(null)
    setStdinValue('')
    sendStdin.mutate(
      { name: sessionName, value },
      { onError: () => toast.error('Could not send input to the runtime.') },
    )
  }

  if (!sessionName) {
    return (
      <section className="rounded-[24px] border border-line bg-paper p-8 text-center">
        <Terminal className="mx-auto size-6 text-graphite" />
        <p className="mt-3 text-sm text-graphite">Select a runtime to run code.</p>
      </section>
    )
  }

  return (
    <section aria-label="Console" className="rounded-[24px] border border-ink bg-paper p-6">
      <div className="flex items-center justify-between gap-3">
        <div className="min-w-0">
          <p className="font-medium">Console</p>
          <p className="truncate text-xs text-graphite">{sessionName}</p>
        </div>
        <span className="shrink-0 font-mono text-xs text-graphite">{cellLabel}</span>
      </div>

      <label className="mt-4 block">
        <span className="sr-only">Code</span>
        <textarea
          value={code}
          onChange={(event) => setCode(event.target.value)}
          spellCheck={false}
          rows={6}
          className="w-full resize-y rounded-2xl border border-ink bg-transparent p-4 font-mono text-sm outline-none placeholder:text-graphite/60 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ink"
        />
      </label>

      <div className="mt-3 flex gap-2">
        <Button size="sm" onClick={run} disabled={running}>
          <Play className="size-4" /> {running ? 'Running…' : 'Run cell'}
        </Button>
        <Button variant="secondary" size="sm" onClick={interrupt} disabled={!running}>
          <Square className="size-4" /> Interrupt
        </Button>
      </div>

      <div
        ref={outputRef}
        className="mt-4 max-h-96 overflow-y-auto rounded-2xl border border-line bg-paper-soft p-4 font-mono text-xs leading-relaxed scrollbar-thin"
      >
        {blocks.length === 0 ? (
          <p className="text-graphite">No output yet.</p>
        ) : (
          blocks.map((block) => <Block key={block.id} block={block} />)
        )}

        {stdinPrompt && (
          <div className="mt-3 flex items-center gap-2 border-t border-line pt-3">
            <span className="shrink-0 text-graphite">{stdinPrompt}</span>
            <input
              autoFocus
              value={stdinValue}
              onChange={(event) => setStdinValue(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === 'Enter') answerStdin()
              }}
              placeholder="type your answer and press Enter"
              className="min-w-0 flex-1 rounded-lg border border-ink bg-transparent px-2 py-1 outline-none"
            />
          </div>
        )}

        {driveRequest && (
          <div className="mt-3 rounded-xl border border-ink bg-paper p-3">
            <p className="text-xs">{driveRequest.message}</p>
            <div className="mt-2 flex flex-wrap gap-2">
              {driveRequest.uri && (
                <ExternalLink
                  href={driveRequest.uri}
                  className="inline-flex h-8 items-center rounded-3xl border border-ink px-3 text-xs font-medium transition-colors hover:bg-paper-soft"
                >
                  Approve access
                </ExternalLink>
              )}
              <Button
                variant="secondary"
                size="sm"
                disabled={driveAuthorize.isPending}
                onClick={() =>
                  driveAuthorize.mutate(
                    { name: sessionName, action: 'drive/authorize' },
                    {
                      onSuccess: (result) => {
                        const outcome = result as { success?: boolean }
                        if (!outcome.success) {
                          toast.error('Access still not granted — approve it first.')
                          return
                        }
                        setDriveRequest(null)
                        toast.success('Access granted — the cell resumes.')
                      },
                      onError: (error) =>
                        toast.error(
                          error instanceof Error ? error.message : 'Drive mount still failing.',
                        ),
                    },
                  )
                }
              >
                Continue
              </Button>
            </div>
          </div>
        )}
      </div>
    </section>
  )
}
