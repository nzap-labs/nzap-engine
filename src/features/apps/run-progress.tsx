import { useEffect, useState } from 'react'
import { Check, Cpu, HardDriveDownload, Loader2, Sparkles, type LucideIcon } from 'lucide-react'
import { cn } from '@/lib/cn'
import type { RunState } from './use-app-run'
import { aboutDuration, formatDuration } from './spec'
import type { Estimate } from './store'

type PhaseId = 'runtime' | 'setup' | 'run'

interface Phase {
  id: PhaseId
  label: string
  icon: LucideIcon
  /** Seconds expected; 0 hides the estimate. */
  expected: number
  startedAt: number | null
  endedAt: number | null
  /** 0–1 reported by the app itself, when it does. */
  reported: number | null
  skipped?: boolean
}

export interface RuntimeStart {
  startedAt: number
  finishedAt: number | null
  expected: number
}

/** Re-render on a timer while something is in flight. */
function useNow(active: boolean): number {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    if (!active) return
    const timer = setInterval(() => setNow(Date.now()), 250)
    return () => clearInterval(timer)
  }, [active])
  return now
}

/**
 * Where a run is: starting the runtime, setting the app up (install,
 * download, load) and generating, each against its estimate.
 */
export function RunProgress({
  state,
  estimate,
  runtimeStart,
}: {
  state: RunState
  estimate: Estimate
  runtimeStart: RuntimeStart | null
}) {
  const active =
    state.status === 'uploading' ||
    state.status === 'running' ||
    Boolean(runtimeStart && !runtimeStart.finishedAt)
  const now = useNow(active)
  const finished = state.finishedAt
  const current = state.stages.at(-1)
  const runStage = current?.id === 'run' ? current : null

  const phases: Phase[] = []
  if (runtimeStart)
    phases.push({
      id: 'runtime',
      label: 'Runtime',
      icon: Cpu,
      expected: runtimeStart.expected,
      startedAt: runtimeStart.startedAt,
      endedAt: runtimeStart.finishedAt,
      reported: null,
    })
  phases.push(
    {
      id: 'setup',
      label: state.warm ? 'Model loaded' : 'Set up',
      icon: HardDriveDownload,
      expected: estimate.setup,
      startedAt: state.startedAt,
      endedAt: state.readyAt ?? (state.status === 'running' || !finished ? null : finished),
      reported: null,
      skipped: state.warm === true || (estimate.setup === 0 && state.status === 'idle'),
    },
    {
      id: 'run',
      label: 'Generate',
      icon: Sparkles,
      expected: estimate.run,
      startedAt: state.readyAt,
      endedAt: state.readyAt && finished ? finished : null,
      reported: runStage?.progress ?? null,
    },
  )

  const message =
    state.status === 'uploading'
      ? 'Uploading your files to the runtime…'
      : state.status === 'running'
        ? (current?.label ?? 'Starting…') + '…'
        : runtimeStart && !runtimeStart.finishedAt
          ? 'Waiting for Google to assign the runtime…'
          : null

  return (
    <div aria-live="polite" className="space-y-3">
      <ol className="grid gap-2" style={{ gridTemplateColumns: `repeat(${phases.length}, 1fr)` }}>
        {phases.map((phase) => (
          <PhaseCell key={phase.id} phase={phase} now={now} failed={state.status === 'error'} />
        ))}
      </ol>
      {message && (
        <p className="flex items-center gap-2 text-sm text-graphite">
          <Loader2 className="size-4 animate-spin" /> {message}
        </p>
      )}
    </div>
  )
}

function PhaseCell({ phase, now, failed }: { phase: Phase; now: number; failed: boolean }) {
  const started = phase.startedAt !== null
  const done = phase.endedAt !== null || phase.skipped
  const running = started && !done && !failed
  const elapsed = started ? ((phase.endedAt ?? now) - phase.startedAt!) / 1000 : 0
  const fraction = done
    ? 1
    : phase.reported !== null
      ? phase.reported
      : phase.expected > 0
        ? Math.min(0.95, elapsed / phase.expected)
        : running
          ? 0.5
          : 0
  const Icon = done ? Check : phase.icon

  return (
    <li className="min-w-0 rounded-2xl border border-line p-3">
      <div className="flex items-center gap-2 text-sm font-medium">
        <Icon className={cn('size-4 shrink-0', done ? 'text-mint' : 'text-graphite')} />
        <span className="truncate">{phase.label}</span>
      </div>
      <div className="mt-2 h-1.5 overflow-hidden rounded-full bg-ink/10">
        <div
          className={cn(
            'h-full rounded-full transition-[width] duration-300',
            failed && !done ? 'bg-coral' : done ? 'bg-mint' : 'bg-sunshine',
          )}
          style={{ width: `${Math.round(fraction * 100)}%` }}
        />
      </div>
      <p className="mt-1.5 text-xs tabular-nums text-graphite">
        {phase.skipped
          ? 'Already warm'
          : done
            ? formatDuration(elapsed)
            : running
              ? `${formatDuration(elapsed)}${phase.expected ? ` of ${aboutDuration(phase.expected)}` : ''}`
              : phase.expected
                ? aboutDuration(phase.expected)
                : '—'}
      </p>
    </li>
  )
}
