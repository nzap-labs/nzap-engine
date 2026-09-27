import { useQuery } from '@tanstack/react-query'
import { useState } from 'react'
import type { ReactNode } from 'react'
import { FolderOpen, RotateCcw, Save } from 'lucide-react'
import { toast } from 'sonner'
import {
  appInfoQuery,
  openLogFolder,
  settingsQuery,
  useSetOAuthClient,
  useUpdateSettings,
  type SettingsPatch,
  type SettingsView,
} from '@/api/app'
import { ExternalLink } from '@/components/external-link'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { PageHeader } from '@/features/shell/page-header'
import { errorMessage } from '@/lib/ipc'

/** Engine settings: keep-alive, the notebook catalog, artifacts, OAuth client. */
export function SettingsPage() {
  const { data: view } = useQuery(settingsQuery)
  if (!view) {
    return (
      <div className="flex min-h-0 flex-1 flex-col">
        <PageHeader title="Settings" />
        <div className="mx-auto mt-6 h-40 w-full max-w-3xl animate-pulse rounded-[24px] bg-paper-soft" />
      </div>
    )
  }
  // Re-seed the form fields whenever the saved settings change.
  return <SettingsForm key={JSON.stringify(view.settings)} view={view} />
}

function SettingsForm({ view }: { view: SettingsView }) {
  const { data: info } = useQuery(appInfoQuery)
  const update = useUpdateSettings()
  const setClient = useSetOAuthClient()
  const settings = view.settings

  const [catalogUrl, setCatalogUrl] = useState(settings.catalogUrl)
  const [interval, setIntervalSeconds] = useState(String(settings.keepAliveIntervalSeconds))
  const [artifactsDir, setArtifactsDir] = useState(settings.artifactsDir ?? '')
  const [clientJson, setClientJson] = useState('')

  function save(patch: SettingsPatch, message = 'Settings saved.') {
    update.mutate(patch, {
      onSuccess: () => toast.success(message),
      onError: (error) => toast.error(errorMessage(error, 'Could not save settings.')),
    })
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <PageHeader title="Settings" />
      <div className="scrollbar-thin min-h-0 flex-1 overflow-y-auto px-4 pb-10 md:px-6">
        <div className="mx-auto w-full max-w-3xl space-y-6">
          <Card
            title="Keep-alive"
            description="Ping every runtime so Colab does not stop it for being idle (for at most 24 hours, like the Colab CLI)."
          >
            <label className="flex items-center gap-2.5 text-sm">
              <input
                type="checkbox"
                checked={settings.keepAlive}
                onChange={(event) => save({ keepAlive: event.target.checked })}
                className="size-4 accent-[var(--color-ink)]"
              />
              Keep runtimes alive while NZAP Engine is open
            </label>
            <form
              className="mt-4 flex flex-wrap items-end gap-2"
              onSubmit={(event) => {
                event.preventDefault()
                save({ keepAliveIntervalSeconds: Number(interval) })
              }}
            >
              <label className="block">
                <span className="text-xs font-medium uppercase tracking-[0.14em] text-graphite">
                  Interval (seconds, 30–600)
                </span>
                <Input
                  type="number"
                  min={30}
                  max={600}
                  value={interval}
                  onChange={(event) => setIntervalSeconds(event.target.value)}
                  className="mt-1.5 w-40"
                />
              </label>
              <Button type="submit" variant="secondary" size="sm" disabled={update.isPending}>
                <Save className="size-4" /> Save
              </Button>
            </form>
          </Card>

          <Card
            title="Public notebook collection"
            description="Where the public notebooks come from. Point this at your own fork of nzap-notebooks to curate a private collection."
          >
            <form
              className="flex flex-wrap gap-2"
              onSubmit={(event) => {
                event.preventDefault()
                save({ catalogUrl }, 'Catalog changed — refresh the collection to load it.')
              }}
            >
              <Input
                value={catalogUrl}
                onChange={(event) => setCatalogUrl(event.target.value)}
                aria-label="Catalog URL"
                className="min-w-0 flex-1 font-mono text-sm"
              />
              <Button type="submit" variant="secondary" size="sm" disabled={update.isPending}>
                <Save className="size-4" /> Save
              </Button>
              <Button
                variant="ghost"
                size="sm"
                disabled={catalogUrl === view.defaultCatalogUrl}
                onClick={() => save({ catalogUrl: view.defaultCatalogUrl }, 'Catalog reset.')}
              >
                <RotateCcw className="size-4" /> Default
              </Button>
            </form>
          </Card>

          <Card
            title="Job artifacts"
            description="Files returned by ephemeral jobs are saved here, in a folder per job. Leave empty for your Downloads folder."
          >
            <form
              className="flex flex-wrap gap-2"
              onSubmit={(event) => {
                event.preventDefault()
                save({ artifactsDir })
              }}
            >
              <Input
                value={artifactsDir}
                onChange={(event) => setArtifactsDir(event.target.value)}
                placeholder="Downloads/NZAP Engine"
                aria-label="Artifacts folder"
                className="min-w-0 flex-1 font-mono text-sm"
              />
              <Button type="submit" variant="secondary" size="sm" disabled={update.isPending}>
                <Save className="size-4" /> Save
              </Button>
            </form>
          </Card>

          <Card
            title="Google OAuth client"
            description="NZAP Engine signs in with the installed-app client that Google's own Colab CLI uses. You can use your own Desktop OAuth client instead — disconnect Google first."
          >
            <p className="text-sm">
              Current client:{' '}
              <span className="break-all font-mono text-xs">{view.oauthClientId}</span>{' '}
              {view.customOauthClient ? '(yours)' : '(built-in)'}
            </p>
            <textarea
              value={clientJson}
              onChange={(event) => setClientJson(event.target.value)}
              rows={4}
              spellCheck={false}
              placeholder='{"installed": {"client_id": "…", "client_secret": "…"}}'
              aria-label="OAuth client JSON"
              className="mt-3 w-full resize-y rounded-2xl border border-ink bg-transparent p-3 font-mono text-xs outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ink"
            />
            <div className="mt-2 flex flex-wrap gap-2">
              <Button
                size="sm"
                variant="secondary"
                disabled={!clientJson.trim() || setClient.isPending}
                onClick={() =>
                  setClient.mutate(clientJson, {
                    onSuccess: () => {
                      setClientJson('')
                      toast.success('OAuth client saved. Connect Google to use it.')
                    },
                    onError: (error) =>
                      toast.error(errorMessage(error, 'Could not use that client.')),
                  })
                }
              >
                <Save className="size-4" /> Use this client
              </Button>
              {view.customOauthClient && (
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={setClient.isPending}
                  onClick={() =>
                    setClient.mutate(null, {
                      onSuccess: () => toast.success('Back to the built-in client.'),
                      onError: (error) =>
                        toast.error(errorMessage(error, 'Could not reset the client.')),
                    })
                  }
                >
                  <RotateCcw className="size-4" /> Use the built-in client
                </Button>
              )}
            </div>
          </Card>

          <Card
            title="About"
            description="NZAP Engine is open source under the Apache-2.0 license."
          >
            <dl className="grid gap-2 text-sm sm:grid-cols-2">
              <div>
                <dt className="text-xs text-graphite">Version</dt>
                <dd className="font-mono">{info?.version ?? '—'}</dd>
              </div>
              <div>
                <dt className="text-xs text-graphite">Platform</dt>
                <dd className="font-mono">{info ? `${info.os} · ${info.arch}` : '—'}</dd>
              </div>
            </dl>
            <div className="mt-4 flex flex-wrap gap-2">
              <Button
                variant="secondary"
                size="sm"
                onClick={() =>
                  openLogFolder().catch((error: unknown) =>
                    toast.error(errorMessage(error, 'Could not open the log folder.')),
                  )
                }
              >
                <FolderOpen className="size-4" /> Open log folder
              </Button>
              <ExternalLink
                href="https://github.com/nzap-labs/nzap-engine/issues/new/choose"
                className="inline-flex h-9 items-center rounded-3xl px-4 text-sm font-medium transition-colors hover:bg-paper-soft"
              >
                Report an issue
              </ExternalLink>
            </div>
          </Card>
        </div>
      </div>
    </div>
  )
}

function Card({
  title,
  description,
  children,
}: {
  title: string
  description: string
  children: ReactNode
}) {
  return (
    <section aria-label={title} className="rounded-[24px] border border-ink bg-paper p-6">
      <p className="font-medium">{title}</p>
      <p className="mt-1 text-sm leading-relaxed text-graphite">{description}</p>
      <div className="mt-4">{children}</div>
    </section>
  )
}
