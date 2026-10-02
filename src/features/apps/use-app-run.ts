import { useCallback, useEffect, useRef, useState } from 'react'
import { streamNotebookRun } from '@/api/notebooks'
import { call, errorMessage } from '@/lib/ipc'
import type { ColabFileModel, ColabSession } from '@/types/colab'
import type { AppEvent, AppOutputPayload } from '@/types/app'
import type { Notebook, NotebookParamValues } from '@/types/notebook'
import { appEventOf, contentsPath, inputUploadPath } from './spec'
import { addResult, markWarm, recordTiming, type ResolvedOutput } from './store'

export type RunStatus = 'idle' | 'uploading' | 'running' | 'done' | 'error' | 'cancelled'

export interface StageView {
  id: string
  label: string
  progress: number | null
  startedAt: number
}

export interface RunState {
  status: RunStatus
  startedAt: number | null
  /** When the notebook reported `ready` (setup finished). */
  readyAt: number | null
  warm: boolean | null
  device: string | null
  stages: StageView[]
  outputs: ResolvedOutput[]
  log: string
  error: string | null
  finishedAt: number | null
}

const IDLE: RunState = {
  status: 'idle',
  startedAt: null,
  readyAt: null,
  warm: null,
  device: null,
  stages: [],
  outputs: [],
  log: '',
  error: null,
  finishedAt: null,
}

const MAX_LOG = 64 * 1024

function base64ToBytes(encoded: string): Uint8Array<ArrayBuffer> {
  const binary = atob(encoded.replace(/\s+/g, ''))
  const bytes = new Uint8Array(binary.length)
  for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index)
  return bytes
}

/** Fetch a media output from the runtime into a blob: URL. */
export async function fetchOutput(
  session: string,
  output: AppOutputPayload,
): Promise<ResolvedOutput> {
  if (!output.path) return output
  try {
    const model = await call<ColabFileModel>('files_read', {
      name: session,
      path: contentsPath(output.path),
    })
    const content = typeof model.content === 'string' ? model.content : ''
    const bytes =
      model.format === 'base64' ? base64ToBytes(content) : new TextEncoder().encode(content)
    const mime = output.mime ?? model.mimetype ?? 'application/octet-stream'
    return { ...output, url: URL.createObjectURL(new Blob([bytes], { type: mime })) }
  } catch (error) {
    return { ...output, fetchError: errorMessage(error, 'Could not fetch the result.') }
  }
}

async function uploadInput(session: string, slug: string, file: File): Promise<string> {
  const target = inputUploadPath(slug, file.name)
  const folders = target.split('/').slice(0, -1)
  for (let depth = 2; depth <= folders.length; depth += 1) {
    // Creating a folder that exists is harmless; a real failure shows on upload.
    await call('files_mkdir', { name: session, path: folders.slice(0, depth).join('/') }).catch(
      () => undefined,
    )
  }
  const bytes = new Uint8Array(await file.arrayBuffer())
  await call('files_upload_bytes', bytes, {
    headers: {
      'x-nzap-session': encodeURIComponent(session),
      'x-nzap-path': encodeURIComponent(target),
    },
  })
  return `/${target}`
}

export interface RunRequest {
  notebook: Notebook
  runtime: ColabSession
  values: NotebookParamValues
  /** Files picked in `file` widgets, by parameter key. */
  files: Record<string, File>
}

/** Run an app on a runtime and follow its progress. */
export function useAppRun() {
  const [state, setState] = useState<RunState>(IDLE)
  const abortRef = useRef<AbortController | null>(null)
  const mounted = useRef(true)

  useEffect(() => {
    mounted.current = true
    return () => {
      mounted.current = false
      abortRef.current?.abort()
    }
  }, [])

  const patch = useCallback((change: (current: RunState) => RunState) => {
    if (mounted.current) setState(change)
  }, [])

  const run = useCallback(
    async ({ notebook, runtime, values, files }: RunRequest) => {
      if (abortRef.current) return
      const controller = new AbortController()
      abortRef.current = controller
      const startedAt = Date.now()
      const pending: Promise<ResolvedOutput>[] = []
      let timing: { setup?: number; run?: number } = {}
      let warm = false
      // Set from inside the event callback, so not a narrowed local.
      const outcome: { failure: string | null } = { failure: null }
      setState({ ...IDLE, status: 'uploading', startedAt })

      try {
        const resolved: NotebookParamValues = { ...values }
        for (const [key, file] of Object.entries(files)) {
          resolved[key] = await uploadInput(runtime.name, notebook.slug, file)
        }
        patch((current) => ({ ...current, status: 'running' }))

        const onApp = (event: AppEvent) => {
          switch (event.event) {
            case 'stage':
              patch((current) => {
                const last = current.stages.at(-1)
                const stage = {
                  id: event.id,
                  label: event.label,
                  progress: event.progress ?? null,
                  startedAt: Date.now(),
                }
                const stages =
                  last && last.id === event.id && last.label === event.label
                    ? [...current.stages.slice(0, -1), { ...last, progress: stage.progress }]
                    : [...current.stages, stage]
                return { ...current, stages }
              })
              break
            case 'ready':
              warm = event.warm
              markWarm(notebook.slug, runtime, true)
              patch((current) => ({
                ...current,
                readyAt: Date.now(),
                warm: event.warm,
                device: event.device ?? null,
              }))
              break
            case 'output': {
              const fetching = fetchOutput(runtime.name, event).then((output) => {
                patch((current) => ({ ...current, outputs: [...current.outputs, output] }))
                return output
              })
              pending.push(fetching)
              break
            }
            case 'done':
              timing = event.seconds ?? {}
              break
          }
        }

        await streamNotebookRun(
          notebook.id,
          { session: runtime.name, params: resolved },
          {
            signal: controller.signal,
            onEvent: (event) => {
              const app = appEventOf(event)
              if (app) return onApp(app)
              if (event.type === 'stream' && event.text) {
                const text = event.text
                patch((current) => ({ ...current, log: (current.log + text).slice(-MAX_LOG) }))
              } else if (event.type === 'error') {
                outcome.failure = `${event.ename ?? 'Error'}: ${event.evalue ?? ''}`.trim()
              } else if (event.type === 'execute_reply' && event.status && event.status !== 'ok') {
                outcome.failure ??= 'The app stopped with an error.'
              }
            },
          },
        )
        const outputs = await Promise.all(pending)
        if (outcome.failure) throw new Error(outcome.failure)

        const seconds = (Date.now() - startedAt) / 1000
        recordTiming(notebook.slug, runtime.accelerator, timing, warm)
        addResult({
          id: crypto.randomUUID(),
          appId: notebook.id,
          startedAt,
          seconds,
          warm,
          runtime: runtime.name,
          accelerator: runtime.accelerator,
          values: resolved,
          outputs,
        })
        patch((current) => ({ ...current, status: 'done', finishedAt: Date.now() }))
      } catch (error) {
        const cancelled = controller.signal.aborted
        // A failed or cancelled run may have left the kernel half-loaded.
        if (!warm) markWarm(notebook.slug, runtime, false)
        patch((current) => ({
          ...current,
          status: cancelled ? 'cancelled' : 'error',
          error: cancelled ? null : errorMessage(error, 'The app failed.'),
          finishedAt: Date.now(),
        }))
      } finally {
        abortRef.current = null
      }
    },
    [patch],
  )

  const cancel = useCallback(() => abortRef.current?.abort(), [])
  const reset = useCallback(() => {
    if (!abortRef.current) setState(IDLE)
  }, [])

  return { state, run, cancel, reset }
}
