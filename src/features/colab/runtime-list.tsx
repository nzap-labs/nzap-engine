import { useQuery } from '@tanstack/react-query'
import { Download, ExternalLink, Import, Plug, RotateCcw, Trash2, Zap } from 'lucide-react'
import { toast } from 'sonner'
import {
  colabAssignmentsQuery,
  colabSessionsQuery,
  useAssignmentAction,
  useDeleteSession,
  useSessionAction,
} from '@/api/colab'
import { Button } from '@/components/ui/button'
import { useDialogs } from '@/components/dialogs'
import { ExternalLink as LinkOut } from '@/components/external-link'
import { cn } from '@/lib/cn'
import type { ColabAssignment, ColabSession } from '@/types/colab'

function formatDuration(totalSeconds: number): string {
  if (!Number.isFinite(totalSeconds)) return '—'
  const hours = Math.floor(totalSeconds / 3600)
  const minutes = Math.floor((totalSeconds % 3600) / 60)
  if (hours > 0) return `${hours}h ${minutes}m`
  if (minutes > 0) return `${minutes}m`
  return `${Math.max(0, Math.floor(totalSeconds))}s`
}

function acceleratorClass(accelerator: string): string {
  if (accelerator === 'CPU' || accelerator === 'NONE') return 'text-graphite'
  if (/^V\d/.test(accelerator)) return 'text-coral'
  return 'text-ink'
}

/** The user's runtimes plus any VMs Google holds that can be imported. */
export function RuntimeList({
  activeName,
  onSelect,
}: {
  activeName: string | null
  onSelect: (name: string) => void
}) {
  const { data: sessionsData, isPending } = useQuery(colabSessionsQuery)
  const { data: assignmentsData } = useQuery(colabAssignmentsQuery)
  const action = useSessionAction()
  const remove = useDeleteSession()
  const assignmentAction = useAssignmentAction()
  const dialogs = useDialogs()

  const sessions = sessionsData?.sessions ?? []
  const assignments = (assignmentsData?.assignments ?? []).filter((item) => !item.managed)

  return (
    <div className="space-y-3">
      <section aria-label="Runtimes" className="rounded-[24px] border border-ink bg-paper p-6">
        <div className="flex items-center justify-between">
          <p className="font-medium">Runtimes</p>
          <span className="text-xs text-graphite">{sessions.length} active</span>
        </div>

        {isPending ? (
          <div className="mt-4 space-y-3">
            <div className="h-20 animate-pulse rounded-2xl bg-paper-soft" />
            <div className="h-20 animate-pulse rounded-2xl bg-paper-soft" />
          </div>
        ) : sessions.length === 0 ? (
          <p className="mt-4 text-sm text-graphite">
            No runtimes yet — launch one and it will show up here.
          </p>
        ) : (
          <ul className="mt-4 space-y-3">
            {[...sessions]
              .sort((a, b) => b.createdAt - a.createdAt)
              .map((session) => (
                <li key={session.name}>
                  <RuntimeCard
                    session={session}
                    active={session.name === activeName}
                    onSelect={() => onSelect(session.name)}
                    busy={action.isPending || remove.isPending}
                    onAction={(kind) => {
                      action.mutate(
                        { name: session.name, action: kind },
                        {
                          onSuccess: () => toast.success(`${kind} → ${session.name}`),
                          onError: (error) =>
                            toast.error(
                              error instanceof Error ? error.message : `Could not ${kind}.`,
                            ),
                        },
                      )
                    }}
                    onDelete={async () => {
                      const confirmed = await dialogs.confirm({
                        title: `Stop ${session.name}?`,
                        description:
                          'The kernel stops and the VM is released back to Google. Files on the runtime are lost.',
                        confirmLabel: 'Stop and release',
                        danger: true,
                      })
                      if (!confirmed) return
                      remove.mutate(session.name, {
                        onSuccess: (outcome) =>
                          outcome.warning
                            ? toast.warning(outcome.warning)
                            : toast.success(`Released ${session.name}.`),
                        onError: (error) =>
                          toast.error(
                            error instanceof Error ? error.message : 'Could not release the VM.',
                          ),
                      })
                    }}
                  />
                </li>
              ))}
          </ul>
        )}
      </section>

      {assignments.length > 0 && (
        <section
          aria-label="External runtimes"
          className="rounded-[24px] border border-line bg-paper p-6"
        >
          <p className="font-medium">External runtimes</p>
          <p className="mt-1 text-sm text-graphite">
            VMs your Google account holds that were created outside NZAP Engine.
          </p>
          <ul className="mt-4 space-y-3">
            {assignments.map((assignment) => (
              <li
                key={assignment.endpoint}
                className="flex flex-wrap items-center justify-between gap-3 rounded-2xl border border-line bg-paper-soft p-4"
              >
                <div className="min-w-0">
                  <p className="truncate font-mono text-xs">{assignment.endpoint}</p>
                  <p className="mt-1 text-xs text-graphite">
                    <span className={cn('font-medium', acceleratorClass(assignment.accelerator))}>
                      {assignment.accelerator}
                    </span>{' '}
                    · {assignment.shape}
                  </p>
                </div>
                <div className="flex gap-2">
                  <Button
                    variant="secondary"
                    size="sm"
                    disabled={assignmentAction.isPending}
                    onClick={() =>
                      assignmentAction.mutate(
                        { action: 'adopt', endpoint: assignment.endpoint },
                        {
                          onSuccess: () => toast.success('Runtime imported.'),
                          onError: (error) =>
                            toast.error(
                              error instanceof Error ? error.message : 'Could not import.',
                            ),
                        },
                      )
                    }
                  >
                    <Import className="size-4" /> Import
                  </Button>
                  <Button
                    variant="danger"
                    size="sm"
                    disabled={assignmentAction.isPending}
                    onClick={async () => {
                      const confirmed = await dialogs.confirm({
                        title: 'Release this VM?',
                        description: `${assignment.endpoint} is released back to Google.`,
                        confirmLabel: 'Release',
                        danger: true,
                      })
                      if (!confirmed) return
                      assignmentAction.mutate(
                        { action: 'release', endpoint: assignment.endpoint },
                        {
                          onSuccess: () => toast.success('Assignment released.'),
                          onError: (error) =>
                            toast.error(
                              error instanceof Error ? error.message : 'Could not release.',
                            ),
                        },
                      )
                    }}
                  >
                    <Download className="size-4 rotate-180" /> Release
                  </Button>
                </div>
              </li>
            ))}
          </ul>
        </section>
      )}
    </div>
  )
}

function RuntimeCard({
  session,
  active,
  onSelect,
  onAction,
  onDelete,
  busy,
}: {
  session: ColabSession
  active: boolean
  onSelect: () => void
  onAction: (kind: 'connect' | 'restart' | 'keepalive') => void
  onDelete: () => void
  busy: boolean
}) {
  return (
    <div
      className={cn(
        'rounded-2xl border p-4 transition-colors',
        active ? 'border-ink bg-paper-soft' : 'border-line bg-paper hover:bg-paper-soft',
      )}
    >
      <button type="button" onClick={onSelect} className="w-full cursor-pointer text-left">
        <div className="flex flex-wrap items-center gap-2">
          <span className="font-medium">{session.name}</span>
          <span
            className={cn(
              'rounded-full border px-2 py-0.5 text-[11px] font-medium uppercase tracking-wide',
              session.connected ? 'border-mint text-ink' : 'border-line text-graphite',
            )}
          >
            {session.connected ? `kernel ${session.kernelState ?? 'live'}` : 'not connected'}
          </span>
          <span className={cn('text-xs font-medium', acceleratorClass(session.accelerator))}>
            {session.accelerator}
          </span>
          <span className="text-xs text-graphite">{session.shape}</span>
        </div>
        <p className="mt-1.5 text-xs text-graphite">
          up {formatDuration(session.uptimeSeconds)} · idle {formatDuration(session.idleSeconds)}
          {session.keepaliveError ? ' · keep-alive failing' : ''}
        </p>
        <LifetimeBar remaining={session.lifetimeRemainingSeconds} uptime={session.uptimeSeconds} />
      </button>
      <div className="mt-3 flex flex-wrap gap-2">
        <Button variant="secondary" size="sm" disabled={busy} onClick={() => onAction('connect')}>
          <Plug className="size-4" /> Connect
        </Button>
        <Button variant="secondary" size="sm" disabled={busy} onClick={() => onAction('restart')}>
          <RotateCcw className="size-4" /> Restart
        </Button>
        <Button variant="ghost" size="sm" disabled={busy} onClick={() => onAction('keepalive')}>
          <Zap className="size-4" /> Keep alive
        </Button>
        {session.colabUrl && (
          <LinkOut
            href={session.colabUrl}
            className="inline-flex h-8 items-center gap-1.5 rounded-3xl px-3 text-sm font-medium transition-colors hover:bg-paper-soft"
          >
            <ExternalLink className="size-4" /> Open in Colab
          </LinkOut>
        )}
        <Button variant="danger" size="sm" disabled={busy} onClick={onDelete}>
          <Trash2 className="size-4" /> Stop
        </Button>
      </div>
    </div>
  )
}

/** How much of Colab's per-VM lifetime ceiling is used up. */
function LifetimeBar({ remaining, uptime }: { remaining: number; uptime: number }) {
  const total = remaining + uptime
  if (!Number.isFinite(total) || total <= 0) return null
  const used = Math.min(100, (uptime / total) * 100)
  return (
    <div className="mt-2">
      <div
        className="h-1 overflow-hidden rounded-full bg-paper-soft"
        role="progressbar"
        aria-label="Runtime lifetime used"
        aria-valuenow={Math.round(used)}
        aria-valuemin={0}
        aria-valuemax={100}
      >
        <div
          className={cn('h-full rounded-full', used > 85 ? 'bg-coral' : 'bg-ink')}
          style={{ width: `${used}%` }}
        />
      </div>
      <p className="mt-1 text-[11px] text-graphite">{formatDuration(remaining)} of lifetime left</p>
    </div>
  )
}

export type { ColabAssignment }
