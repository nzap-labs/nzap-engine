import { useRef, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Check, FileUp, FolderOpen, Loader2, Rocket, Square } from 'lucide-react'
import { toast } from 'sonner'
import {
  colabConfigQuery,
  colabQuotaQuery,
  colabStatusQuery,
  revealPath,
  streamJob,
  useDeleteSession,
} from '@/api/colab'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/cn'
import { errorMessage } from '@/lib/ipc'
import type { ColabJobEvent } from '@/types/colab'
import { Block, renderMimeBundle, type OutputBlock } from './output-view'

type Hardware = 'cpu' | 'gpu' | 'tpu'
type Phase = 'assigning' | 'connecting' | 'running' | 'collecting' | 'released'

const PHASES: { id: Phase; label: string }[] = [
  { id: 'assigning', label: 'Allocate VM' },
  { id: 'connecting', label: 'Connect kernel' },
  { id: 'running', label: 'Run script' },
  { id: 'collecting', label: 'Collect artifacts' },
  { id: 'released', label: 'Release VM' },
]

const FIELD =
  'w-full rounded-2xl border border-ink bg-transparent px-4 text-sm outline-none placeholder:text-graphite/60 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ink'

let nextId = 1

/**
 * `colab run SCRIPT [ARGS…]` as a form: a fresh VM runs one script with
 * `python script.py` semantics, hands back the files it produced, and is
 * released — no runtime to manage by hand.
 */
export function JobsPanel() {
  const queryClient = useQueryClient()
  const { data: status } = useQuery(colabStatusQuery)
  const { data: config } = useQuery(colabConfigQuery)
  const { data: quota } = useQuery({ ...colabQuotaQuery, enabled: Boolean(status?.connected) })
  const release = useDeleteSession()

  const [script, setScript] = useState('import sys\nprint("args:", sys.argv[1:])\n')
  const [filename, setFilename] = useState('job.py')
  const [args, setArgs] = useState('')
  const [env, setEnv] = useState('')
  const [artifacts, setArtifacts] = useState('')
  const [hardware, setHardware] = useState<Hardware>('cpu')
  const [accelerator, setAccelerator] = useState('')
  const [highMem, setHighMem] = useState(false)
  const [keep, setKeep] = useState(false)

  const [running, setRunning] = useState(false)
  const [session, setSession] = useState<string | null>(null)
  const [phases, setPhases] = useState<Phase[]>([])
  const [blocks, setBlocks] = useState<OutputBlock[]>([])
  const [files, setFiles] = useState<
    { path: string; size: number; savedTo?: string; skipped?: string }[]
  >([])
  const [done, setDone] = useState<{ exitCode: number; error?: string; kept?: boolean } | null>(
    null,
  )
  const abortRef = useRef<AbortController | null>(null)
  const picker = useRef<HTMLInputElement>(null)

  const ineligible = new Set(quota?.ineligibleAccelerators ?? [])
  const options =
    hardware === 'gpu' ? (config?.gpus ?? []) : hardware === 'tpu' ? (config?.tpus ?? []) : []
  const eligible = (option: string) => !ineligible.has(option.toUpperCase())
  const selected =
    options.includes(accelerator) && eligible(accelerator)
      ? accelerator
      : (options.find(eligible) ?? '')

  function append(block: Omit<OutputBlock, 'id'>) {
    setBlocks((current) => [...current, { ...block, id: nextId++ }])
  }

  function onEvent(event: ColabJobEvent) {
    switch (event.type) {
      case 'job':
        setSession(event.session)
        setPhases((current) => [...current, event.phase])
        break
      case 'stream':
        append({ kind: 'stream', streamName: event.name ?? 'stdout', text: event.text ?? '' })
        break
      case 'result':
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
      case 'artifact':
        setFiles((current) => [
          ...current,
          { path: event.path, size: event.size, savedTo: event.savedTo, skipped: event.skipped },
        ])
        break
      case 'job_done':
        setDone({ exitCode: event.exit_code, error: event.error, kept: event.kept })
        break
      default:
        break
    }
  }

  async function pick(file: File | undefined) {
    if (!file) return
    if (!file.name.endsWith('.py')) {
      toast.error('Jobs run Python scripts (.py).')
      return
    }
    setFilename(file.name)
    setScript(await file.text())
  }

  async function launch() {
    if (running || !script.trim()) return
    setRunning(true)
    setSession(null)
    setPhases([])
    setBlocks([])
    setFiles([])
    setDone(null)
    const controller = new AbortController()
    abortRef.current = controller
    try {
      await streamJob(
        {
          filename,
          script,
          args: splitArgs(args),
          env: lines(env),
          artifacts: lines(artifacts),
          gpu: hardware === 'gpu' ? selected : undefined,
          tpu: hardware === 'tpu' ? selected : undefined,
          highMem: highMem || undefined,
          keep,
        },
        { onEvent, signal: controller.signal },
      )
    } catch (error) {
      if (!controller.signal.aborted) {
        toast.error(error instanceof Error ? error.message : 'The job failed.')
      }
    } finally {
      abortRef.current = null
      setRunning(false)
      void queryClient.invalidateQueries({ queryKey: ['colab'] })
    }
  }

  function cancel() {
    // Cancelling the stream makes the engine release the VM (a guard tears it
    // down); release explicitly too in case the job was kept.
    abortRef.current?.abort()
    if (session) release.mutate(session)
  }

  const noAccelerator = hardware !== 'cpu' && !selected
  const disabled = !status?.connected || running || noAccelerator || !script.trim()

  return (
    <section aria-label="Jobs" className="rounded-[24px] border border-ink bg-paper p-6">
      <p className="font-medium">Ephemeral job</p>
      <p className="mt-1 text-sm text-graphite">
        A fresh runtime runs one script like <span className="font-mono">python job.py ARGS</span>,
        saves the files you name to your downloads folder, and is released — the desktop version of{' '}
        <span className="font-mono">colab run</span>.
      </p>

      <div className="mt-4 space-y-3">
        <div className="flex flex-wrap items-center gap-2">
          <input
            ref={picker}
            type="file"
            accept=".py"
            className="hidden"
            onChange={(event) => void pick(event.target.files?.[0])}
          />
          <Button variant="secondary" size="sm" onClick={() => picker.current?.click()}>
            <FileUp className="size-4" /> Load .py
          </Button>
          <input
            value={filename}
            onChange={(event) => setFilename(event.target.value)}
            aria-label="Script file name"
            className={cn(FIELD, 'h-9 w-40 font-mono text-xs')}
          />
        </div>
        <textarea
          value={script}
          onChange={(event) => setScript(event.target.value)}
          rows={8}
          spellCheck={false}
          aria-label="Script"
          className={cn(FIELD, 'resize-y p-4 font-mono')}
        />

        <div className="grid gap-3 md:grid-cols-2">
          <label className="block">
            <span className="text-xs font-medium uppercase tracking-[0.14em] text-graphite">
              Arguments
            </span>
            <input
              value={args}
              onChange={(event) => setArgs(event.target.value)}
              placeholder={'--epochs 3 --name "run one"'}
              className={cn(FIELD, 'mt-1.5 h-10 font-mono text-xs')}
            />
          </label>
          <label className="block">
            <span className="text-xs font-medium uppercase tracking-[0.14em] text-graphite">
              Artifacts to return (one per line, under /content)
            </span>
            <textarea
              value={artifacts}
              onChange={(event) => setArtifacts(event.target.value)}
              rows={2}
              placeholder={'out/*.png\ncheckpoints/model.bin'}
              className={cn(FIELD, 'mt-1.5 resize-y p-3 font-mono text-xs')}
            />
          </label>
          <label className="block md:col-span-2">
            <span className="text-xs font-medium uppercase tracking-[0.14em] text-graphite">
              Environment (KEY=VALUE per line)
            </span>
            <textarea
              value={env}
              onChange={(event) => setEnv(event.target.value)}
              rows={2}
              className={cn(FIELD, 'mt-1.5 resize-y p-3 font-mono text-xs')}
            />
          </label>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          {(['cpu', 'gpu', 'tpu'] as Hardware[]).map((option) => (
            <button
              key={option}
              type="button"
              aria-pressed={hardware === option}
              onClick={() => setHardware(option)}
              className={cn(
                'h-9 cursor-pointer rounded-3xl border px-4 text-sm font-medium uppercase transition-colors',
                hardware === option
                  ? 'border-ink bg-sunshine text-on-sunshine'
                  : 'border-ink text-ink hover:bg-paper-soft',
              )}
            >
              {option}
            </button>
          ))}
          {options.length > 0 && (
            <select
              value={selected}
              onChange={(event) => setAccelerator(event.target.value)}
              aria-label="Accelerator"
              className="h-9 rounded-3xl border border-ink bg-transparent px-3 text-sm outline-none"
            >
              {options.map((option) => (
                <option key={option} value={option} disabled={!eligible(option)}>
                  {option.toUpperCase()}
                  {eligible(option) ? '' : ' — not on your plan'}
                </option>
              ))}
            </select>
          )}
          {hardware !== 'cpu' && (
            <label className="flex items-center gap-2 text-sm">
              <input
                type="checkbox"
                checked={highMem}
                onChange={(event) => setHighMem(event.target.checked)}
                className="size-4 accent-[var(--color-ink)]"
              />
              High-RAM
            </label>
          )}
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={keep}
              onChange={(event) => setKeep(event.target.checked)}
              className="size-4 accent-[var(--color-ink)]"
            />
            Keep the runtime afterwards
          </label>
        </div>

        <div className="flex gap-2">
          <Button onClick={() => void launch()} disabled={disabled}>
            {running ? <Loader2 className="size-4 animate-spin" /> : <Rocket className="size-4" />}
            {running ? 'Running job…' : 'Run job'}
          </Button>
          <Button variant="secondary" onClick={cancel} disabled={!running}>
            <Square className="size-4" /> Cancel
          </Button>
        </div>
        {!status?.connected && <p className="text-xs text-graphite">Connect Google Auth first.</p>}
      </div>

      {(phases.length > 0 || done) && (
        <div className="mt-5 border-t border-line pt-5">
          <ol className="flex flex-wrap gap-2 text-xs">
            {PHASES.filter((phase) => phase.id !== 'collecting' || lines(artifacts).length).map(
              (phase) => {
                const reached = phases.includes(phase.id)
                const current = reached && phases.at(-1) === phase.id && running
                return (
                  <li
                    key={phase.id}
                    className={cn(
                      'flex items-center gap-1.5 rounded-full border px-3 py-1',
                      reached ? 'border-ink' : 'border-line text-graphite',
                    )}
                  >
                    {current ? (
                      <Loader2 className="size-3 animate-spin" />
                    ) : reached ? (
                      <Check className="size-3" />
                    ) : null}
                    {phase.label}
                  </li>
                )
              },
            )}
          </ol>
          {session && <p className="mt-2 font-mono text-xs text-graphite">runtime: {session}</p>}

          {blocks.length > 0 && (
            <div className="mt-3 max-h-80 overflow-y-auto rounded-2xl border border-line bg-paper-soft p-4 font-mono text-xs leading-relaxed scrollbar-thin">
              {blocks.map((block) => (
                <Block key={block.id} block={block} />
              ))}
            </div>
          )}

          {files.length > 0 && (
            <ul className="mt-3 space-y-2">
              {files.map((file) => (
                <li
                  key={file.path}
                  className="flex items-center justify-between gap-3 rounded-xl border border-line px-3 py-2 text-xs"
                >
                  <span className="truncate font-mono">{file.path}</span>
                  {file.savedTo ? (
                    <Button
                      variant="secondary"
                      size="sm"
                      title={file.savedTo}
                      onClick={() =>
                        revealPath(file.savedTo!).catch((error: unknown) =>
                          toast.error(errorMessage(error, 'Could not show the file.')),
                        )
                      }
                    >
                      <FolderOpen className="size-4" /> {formatSize(file.size)}
                    </Button>
                  ) : (
                    <span className="text-graphite">skipped ({file.skipped})</span>
                  )}
                </li>
              ))}
            </ul>
          )}

          {done && (
            <p
              className={cn(
                'mt-3 text-sm font-medium',
                done.exitCode === 0 ? 'text-ink' : 'text-coral',
              )}
            >
              {done.error
                ? done.error
                : `Exit code ${done.exitCode}${done.kept ? ' · runtime kept' : ' · VM released'}`}
            </p>
          )}
        </div>
      )}
    </section>
  )
}

function lines(text: string): string[] {
  return text
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean)
}

/** Shell-like argument splitting: whitespace, with '…' and "…" quoting. */
function splitArgs(text: string): string[] {
  const out: string[] = []
  const pattern = /"([^"]*)"|'([^']*)'|(\S+)/g
  let match: RegExpExecArray | null
  while ((match = pattern.exec(text))) out.push(match[1] ?? match[2] ?? match[3] ?? '')
  return out
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
