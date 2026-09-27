import { useRef, useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { Cloud, FileText, HardDrive, Loader2, Package, X } from 'lucide-react'
import { toast } from 'sonner'
import { streamAutomation, useSessionAction } from '@/api/colab'
import { Button } from '@/components/ui/button'
import { ExternalLink } from '@/components/external-link'
import { cn } from '@/lib/cn'
import type { ColabAutomationOp, ColabAutomationRequest, ColabExecuteEvent } from '@/types/colab'

const FIELD =
  'h-10 w-full rounded-2xl border border-ink bg-transparent px-4 text-sm outline-none placeholder:text-graphite/60 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ink'

/**
 * One-click runtime setup — the CLI's automation commands:
 * `colab install`, `colab drivemount [PATH]` and `colab auth`.
 */
export function SetupCard({ sessionName }: { sessionName: string }) {
  const queryClient = useQueryClient()
  const authorize = useSessionAction()
  const [packages, setPackages] = useState('')
  const [requirements, setRequirements] = useState<{ filename: string; content: string } | null>(
    null,
  )
  const [mountPath, setMountPath] = useState('/content/drive')
  const [running, setRunning] = useState<ColabAutomationOp | null>(null)
  const [log, setLog] = useState<{ text: string; tone: 'out' | 'err' | 'info' }[]>([])
  const [consent, setConsent] = useState<{ message: string; uri?: string } | null>(null)
  const fileInput = useRef<HTMLInputElement>(null)

  function append(text: string, tone: 'out' | 'err' | 'info' = 'out') {
    setLog((current) => [...current, { text, tone }])
  }

  function onEvent(event: ColabExecuteEvent) {
    switch (event.type) {
      case 'stream':
        append(event.text ?? '', event.name === 'stderr' ? 'err' : 'out')
        break
      case 'error':
        append(
          (event.traceback ?? []).join('\n') || `${event.ename ?? 'Error'}: ${event.evalue ?? ''}`,
          'err',
        )
        break
      case 'colab_request':
        append(event.message ?? '', 'info')
        break
      case 'drive_auth_required':
        setConsent({ message: event.message ?? 'Approve access to continue.', uri: event.uri })
        break
      case 'automation':
        if (event.state === 'finished') {
          append(event.status === 'ok' ? '✓ done' : '✗ finished with errors', 'info')
        }
        break
      default:
        break
    }
  }

  async function run(op: ColabAutomationOp, body: ColabAutomationRequest, title: string) {
    if (running) return
    setRunning(op)
    setConsent(null)
    setLog([{ text: title, tone: 'info' }])
    try {
      await streamAutomation(sessionName, op, body, { onEvent })
    } catch (error) {
      append(error instanceof Error ? error.message : 'Automation failed.', 'err')
    } finally {
      setRunning(null)
      setConsent(null)
      void queryClient.invalidateQueries({ queryKey: ['colab', 'history', sessionName] })
      void queryClient.invalidateQueries({ queryKey: ['colab', 'sessions'] })
    }
  }

  function install() {
    const list = packages.split(/[\s,]+/).filter(Boolean)
    if (!list.length && !requirements) {
      toast.error('Add packages or a requirements file.')
      return
    }
    void run(
      'install',
      { packages: list, requirements: requirements ?? undefined },
      `Installing ${[...list, requirements?.filename].filter(Boolean).join(', ')} (uv, pip fallback)…`,
    )
  }

  async function pickRequirements(file: File | undefined) {
    if (!file) return
    if (file.size > 256 * 1024) {
      toast.error('That requirements file is too large.')
      return
    }
    setRequirements({ filename: file.name, content: await file.text() })
  }

  function resume() {
    authorize.mutate(
      { name: sessionName, action: 'drive/authorize' },
      {
        onSuccess: (result) => {
          const outcome = result as { success?: boolean; unauthorized_redirect_uri?: string }
          if (outcome.success) {
            setConsent(null)
            append('Access granted — resuming…', 'info')
          } else {
            setConsent((current) => ({
              message: 'Access still not granted. Approve it in the Google tab first.',
              uri: outcome.unauthorized_redirect_uri ?? current?.uri,
            }))
          }
        },
        onError: (error) =>
          toast.error(error instanceof Error ? error.message : 'Could not continue.'),
      },
    )
  }

  const busy = running !== null

  return (
    <section aria-label="Runtime setup" className="rounded-[24px] border border-line bg-paper p-6">
      <p className="font-medium">Setup</p>
      <p className="mt-1 text-sm text-graphite">
        Install packages, mount Google Drive, or sign the runtime in to Google Cloud.
      </p>

      <div className="mt-4 grid gap-4 md:grid-cols-3">
        <div className="space-y-2 md:col-span-3">
          <span className="flex items-center gap-1.5 text-xs font-medium uppercase tracking-[0.14em] text-graphite">
            <Package className="size-3.5" /> Packages
          </span>
          <div className="flex flex-wrap gap-2">
            <input
              value={packages}
              onChange={(event) => setPackages(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === 'Enter') install()
              }}
              placeholder="torch transformers==4.46 'numpy<2'"
              aria-label="Packages to install"
              className={cn(FIELD, 'min-w-0 flex-1')}
            />
            <input
              ref={fileInput}
              type="file"
              accept=".txt,.in,text/plain"
              className="hidden"
              onChange={(event) => void pickRequirements(event.target.files?.[0])}
            />
            {requirements ? (
              <span className="inline-flex h-10 items-center gap-2 rounded-2xl border border-line px-3 text-xs">
                <FileText className="size-3.5" /> {requirements.filename}
                <button
                  type="button"
                  aria-label="Remove requirements file"
                  className="cursor-pointer"
                  onClick={() => {
                    setRequirements(null)
                    if (fileInput.current) fileInput.current.value = ''
                  }}
                >
                  <X className="size-3.5" />
                </button>
              </span>
            ) : (
              <Button variant="secondary" size="sm" onClick={() => fileInput.current?.click()}>
                <FileText className="size-4" /> requirements.txt
              </Button>
            )}
            <Button size="sm" onClick={install} disabled={busy}>
              {running === 'install' ? (
                <Loader2 className="size-4 animate-spin" />
              ) : (
                <Package className="size-4" />
              )}
              Install
            </Button>
          </div>
        </div>

        <div className="space-y-2 md:col-span-2">
          <span className="flex items-center gap-1.5 text-xs font-medium uppercase tracking-[0.14em] text-graphite">
            <HardDrive className="size-3.5" /> Google Drive
          </span>
          <div className="flex gap-2">
            <input
              value={mountPath}
              onChange={(event) => setMountPath(event.target.value)}
              aria-label="Drive mount path"
              className={cn(FIELD, 'min-w-0 flex-1 font-mono')}
            />
            <Button
              variant="secondary"
              size="sm"
              disabled={busy}
              onClick={() =>
                void run(
                  'drivemount',
                  { path: mountPath },
                  `Mounting Google Drive at ${mountPath}…`,
                )
              }
            >
              {running === 'drivemount' && <Loader2 className="size-4 animate-spin" />}
              Mount
            </Button>
          </div>
        </div>

        <div className="space-y-2">
          <span className="flex items-center gap-1.5 text-xs font-medium uppercase tracking-[0.14em] text-graphite">
            <Cloud className="size-3.5" /> Google Cloud
          </span>
          <Button
            variant="secondary"
            size="sm"
            className="w-full"
            disabled={busy}
            onClick={() =>
              void run('gcp-auth', {}, 'Authenticating the runtime with Google Cloud…')
            }
          >
            {running === 'gcp-auth' && <Loader2 className="size-4 animate-spin" />}
            Authenticate
          </Button>
        </div>
      </div>

      {consent && (
        <div className="mt-4 rounded-xl border border-ink bg-paper-soft p-3">
          <p className="text-xs">{consent.message}</p>
          <div className="mt-2 flex flex-wrap gap-2">
            {consent.uri && (
              <ExternalLink
                href={consent.uri}
                className="inline-flex h-8 items-center rounded-3xl border border-ink px-3 text-xs font-medium transition-colors hover:bg-paper"
              >
                Approve access
              </ExternalLink>
            )}
            <Button size="sm" disabled={authorize.isPending} onClick={resume}>
              Continue
            </Button>
          </div>
        </div>
      )}

      {log.length > 0 && (
        <div className="mt-4 max-h-56 overflow-y-auto rounded-2xl border border-line bg-paper-soft p-3 font-mono text-xs leading-relaxed scrollbar-thin">
          {log.map((line, index) => (
            <pre
              key={index}
              className={cn(
                'whitespace-pre-wrap break-words',
                line.tone === 'err' && 'text-coral',
                line.tone === 'info' && 'text-graphite',
              )}
            >
              {line.text}
            </pre>
          ))}
        </div>
      )}
    </section>
  )
}
