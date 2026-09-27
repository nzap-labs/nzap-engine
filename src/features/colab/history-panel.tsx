import { useQuery } from '@tanstack/react-query'
import { Download, History, RefreshCw, Trash2 } from 'lucide-react'
import { toast } from 'sonner'
import { colabHistoryQuery, useClearHistory, useExportHistory } from '@/api/colab'
import { Button } from '@/components/ui/button'
import { useDialogs } from '@/components/dialogs'
import { cn } from '@/lib/cn'
import type { ColabHistoryEvent, ColabHistoryFormat } from '@/types/colab'

const FORMATS: { id: ColabHistoryFormat; label: string }[] = [
  { id: 'ipynb', label: 'Notebook' },
  { id: 'md', label: 'Markdown' },
  { id: 'txt', label: 'Text' },
  { id: 'jsonl', label: 'JSONL' },
]

/**
 * The runtime's history log (`colab log`): every cell, automation, stdin
 * answer and file operation, exportable as a notebook, Markdown, text or
 * JSONL.
 */
export function HistoryPanel({ sessionName }: { sessionName: string }) {
  const { data, isFetching, refetch } = useQuery(colabHistoryQuery(sessionName))
  const exportHistory = useExportHistory()
  const clear = useClearHistory()
  const dialogs = useDialogs()
  const events = [...(data?.events ?? [])].reverse()

  return (
    <section aria-label="History" className="rounded-[24px] border border-line bg-paper p-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex items-center gap-2">
          <History className="size-4 text-graphite" />
          <p className="font-medium">History</p>
          <span className="text-xs text-graphite">{data?.events.length ?? 0} events</span>
        </div>
        <div className="flex flex-wrap gap-2">
          {FORMATS.map((format) => (
            <Button
              key={format.id}
              variant="secondary"
              size="sm"
              disabled={exportHistory.isPending || !events.length}
              onClick={() =>
                exportHistory.mutate(
                  { name: sessionName, format: format.id },
                  {
                    onSuccess: (path) => path && toast.success(`Saved to ${path}.`),
                    onError: (error) =>
                      toast.error(error instanceof Error ? error.message : 'Export failed.'),
                  },
                )
              }
            >
              <Download className="size-4" /> {format.label}
            </Button>
          ))}
          <Button
            variant="ghost"
            size="sm"
            aria-label="Refresh history"
            onClick={() => void refetch()}
          >
            <RefreshCw className={cn('size-4', isFetching && 'animate-spin')} />
          </Button>
          <Button
            variant="ghost"
            size="sm"
            aria-label="Clear history"
            disabled={clear.isPending || !events.length}
            onClick={async () => {
              const confirmed = await dialogs.confirm({
                title: 'Clear history?',
                description: `Every recorded cell and operation of ${sessionName} is deleted from this computer.`,
                confirmLabel: 'Clear',
                danger: true,
              })
              if (confirmed) clear.mutate(sessionName)
            }}
          >
            <Trash2 className="size-4" />
          </Button>
        </div>
      </div>

      {events.length === 0 ? (
        <p className="mt-4 text-sm text-graphite">Nothing recorded yet — run a cell.</p>
      ) : (
        <ol className="mt-4 max-h-72 space-y-2 overflow-y-auto scrollbar-thin">
          {events.map((event, index) => (
            <li
              key={`${event.timestamp}-${index}`}
              className="rounded-xl border border-line bg-paper-soft px-3 py-2 text-xs"
            >
              <div className="flex items-center justify-between gap-3 text-graphite">
                <span className="font-medium uppercase tracking-wide text-ink">{label(event)}</span>
                <time dateTime={event.timestamp}>
                  {new Date(event.timestamp).toLocaleTimeString()}
                </time>
              </div>
              {detail(event) && (
                <pre className="mt-1 max-h-24 overflow-hidden whitespace-pre-wrap break-words font-mono text-[11px]">
                  {detail(event)}
                </pre>
              )}
            </li>
          ))}
        </ol>
      )}
    </section>
  )
}

function label(event: ColabHistoryEvent): string {
  switch (event.event_type) {
    case 'execution':
      return event.status === 'ok' ? 'cell ✓' : `cell ✗ ${event.status ?? ''}`
    case 'automation':
      return `automation · ${event.op ?? ''}`
    case 'automation_result':
      return `automation done · ${event.op ?? ''}`
    case 'file_operation':
      return `file · ${event.op ?? ''}`
    default:
      return event.event_type.replace(/_/g, ' ')
  }
}

function detail(event: ColabHistoryEvent): string | null {
  if (event.event_type === 'execution' || event.event_type === 'automation') {
    return event.code ?? null
  }
  if (event.event_type === 'file_operation') return event.path ?? null
  if (event.event_type === 'input_reply') return String(event.value ?? '')
  if (event.event_type === 'session_created') {
    return `${event.accelerator ?? ''} · ${event.endpoint ?? ''}`
  }
  if (event.event_type === 'session_terminated') return event.reason ?? null
  return null
}
