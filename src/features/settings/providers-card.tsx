import { useQuery } from '@tanstack/react-query'
import { useRef, useState } from 'react'
import type { ReactNode } from 'react'
import { KeyRound, LogIn, LogOut, Save, Terminal, X } from 'lucide-react'
import { toast } from 'sonner'
import type { SettingsPatch, SettingsView } from '@/api/app'
import {
  providersQuery,
  useCodesandboxDisconnect,
  useCodesandboxKey,
  useGithubCli,
  useGithubDeviceLogin,
  useGithubDisconnect,
  useGithubToken,
  type DeviceCode,
  type ProviderStatus,
} from '@/api/providers'
import { ExternalLink } from '@/components/external-link'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { errorMessage } from '@/lib/ipc'

const TIERS = ['Pico', 'Nano', 'Micro', 'Small', 'Medium', 'Large', 'XLarge']

const fieldLabel = 'text-xs font-medium uppercase tracking-[0.14em] text-graphite'

/**
 * GitHub Codespaces and CodeSandbox as compute next to Colab: connect each
 * account, pick the defaults for new runtimes, and when idle runtimes are
 * deleted.
 */
export function ProvidersCard({
  view,
  save,
  saving,
}: {
  view: SettingsView
  save: (patch: SettingsPatch, message?: string) => void
  saving: boolean
}) {
  const { data: statuses } = useQuery(providersQuery)
  const settings = view.settings
  const github = statuses?.find((status) => status.provider === 'codespaces')
  const codesandbox = statuses?.find((status) => status.provider === 'codesandbox')
  const [minutes, setMinutes] = useState(String(settings.autoReleaseMinutes))

  return (
    <section
      aria-label="Compute providers"
      className="rounded-[24px] border border-ink bg-paper p-6"
    >
      <p className="font-medium">Compute providers</p>
      <p className="mt-1 text-sm leading-relaxed text-graphite">
        Run runtimes on GitHub Codespaces or CodeSandbox as well as Colab. A runtime is created when
        you launch it (or an app or agent needs one), stays warm for reuse, and is deleted when you
        stop it, or automatically once it has been idle.
      </p>

      <div className="mt-5 space-y-6">
        <GithubSection status={github} view={view} save={save} saving={saving} />
        <CodesandboxSection status={codesandbox} view={view} save={save} saving={saving} />

        <form
          className="flex flex-wrap items-end gap-2"
          onSubmit={(event) => {
            event.preventDefault()
            save(
              { autoReleaseMinutes: Number(minutes) },
              Number(minutes) === 0
                ? 'Idle runtimes are no longer deleted automatically.'
                : `Idle runtimes are deleted after ${minutes} minutes.`,
            )
          }}
        >
          <label className="block">
            <span className={fieldLabel}>Delete idle runtimes after (minutes, 0 = never)</span>
            <Input
              type="number"
              min={0}
              max={1440}
              value={minutes}
              onChange={(event) => setMinutes(event.target.value)}
              className="mt-1.5 w-40"
            />
          </label>
          <Button type="submit" variant="secondary" size="sm" disabled={saving}>
            <Save className="size-4" /> Save
          </Button>
        </form>
      </div>
    </section>
  )
}

function Section({
  title,
  status,
  children,
}: {
  title: string
  status: ProviderStatus | undefined
  children: ReactNode
}) {
  const label = status?.connected
    ? status.account
      ? `Connected as ${status.account}`
      : 'Connected'
    : 'Not connected'
  return (
    <div aria-label={title} role="group" className="rounded-2xl border border-line p-4">
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-medium">{title}</span>
        <span
          className={
            status?.connected
              ? 'rounded-full border border-mint px-2 py-0.5 text-[11px] font-medium uppercase tracking-wide'
              : 'rounded-full border border-line px-2 py-0.5 text-[11px] font-medium uppercase tracking-wide text-graphite'
          }
        >
          {label}
        </span>
        {status?.detail && <span className="text-xs text-graphite">{status.detail}</span>}
      </div>
      {status?.error && <p className="mt-2 text-sm text-coral">{status.error}</p>}
      <div className="mt-3 space-y-3">{children}</div>
    </div>
  )
}

function GithubSection({
  status,
  view,
  save,
  saving,
}: {
  status: ProviderStatus | undefined
  view: SettingsView
  save: (patch: SettingsPatch, message?: string) => void
  saving: boolean
}) {
  const settings = view.settings
  const tokenMutation = useGithubToken()
  const cli = useGithubCli()
  const device = useGithubDeviceLogin()
  const disconnect = useGithubDisconnect()
  const [token, setToken] = useState('')
  const [code, setCode] = useState<DeviceCode | null>(null)
  const abort = useRef<AbortController | null>(null)
  const [repository, setRepository] = useState(settings.codespacesRepository)
  const [machine, setMachine] = useState(settings.codespacesMachine)
  const [devcontainer, setDevcontainer] = useState(settings.codespacesDevcontainerPath)
  const [clientId, setClientId] = useState(settings.githubClientId ?? '')

  const connected = Boolean(status?.connected)
  const connectedAs = (login: string) => toast.success(`GitHub connected as ${login}.`)
  const failed = (fallback: string) => (error: unknown) =>
    toast.error(errorMessage(error, fallback))

  function signIn() {
    const controller = new AbortController()
    abort.current = controller
    device.mutate(
      { onCode: setCode, signal: controller.signal },
      {
        onSuccess: (user) => connectedAs(user.login),
        onError: (error) => {
          if (!controller.signal.aborted) failed('GitHub sign-in failed.')(error)
        },
        onSettled: () => {
          setCode(null)
          abort.current = null
        },
      },
    )
  }

  return (
    <Section title="GitHub Codespaces" status={status}>
      {connected ? (
        <Button
          variant="ghost"
          size="sm"
          disabled={disconnect.isPending}
          onClick={() =>
            disconnect.mutate(undefined, {
              onSuccess: () => toast.success('GitHub disconnected.'),
              onError: failed('Could not disconnect GitHub.'),
            })
          }
        >
          <LogOut className="size-4" /> Disconnect GitHub
        </Button>
      ) : (
        <>
          {code ? (
            <div className="rounded-2xl bg-paper-soft p-3 text-sm" role="status">
              Enter <span className="font-mono text-base font-semibold">{code.userCode}</span> at{' '}
              <ExternalLink href={code.verificationUri} className="underline underline-offset-4">
                {code.verificationUri.replace(/^https:\/\//, '')}
              </ExternalLink>
              , then come back.
              <Button
                variant="ghost"
                size="sm"
                className="ml-2"
                onClick={() => abort.current?.abort()}
              >
                <X className="size-4" /> Cancel
              </Button>
            </div>
          ) : (
            <div className="flex flex-wrap gap-2">
              <Button size="sm" disabled={device.isPending} onClick={signIn}>
                <LogIn className="size-4" /> Sign in with GitHub
              </Button>
              <Button
                variant="secondary"
                size="sm"
                disabled={cli.isPending}
                onClick={() =>
                  cli.mutate(undefined, {
                    onSuccess: (user) => connectedAs(user.login),
                    onError: failed('Could not use the GitHub CLI login.'),
                  })
                }
              >
                <Terminal className="size-4" /> Use GitHub CLI login
              </Button>
            </div>
          )}
          <form
            className="flex flex-wrap gap-2"
            onSubmit={(event) => {
              event.preventDefault()
              tokenMutation.mutate(token, {
                onSuccess: (user) => {
                  setToken('')
                  connectedAs(user.login)
                },
                onError: failed('GitHub did not accept that token.'),
              })
            }}
          >
            <Input
              type="password"
              value={token}
              onChange={(event) => setToken(event.target.value)}
              placeholder="or paste a token with the codespace scope"
              aria-label="GitHub token"
              className="min-w-0 flex-1 font-mono text-sm"
            />
            <Button
              type="submit"
              variant="secondary"
              size="sm"
              disabled={!token.trim() || tokenMutation.isPending}
            >
              <KeyRound className="size-4" /> Use token
            </Button>
          </form>
        </>
      )}

      <form
        className="grid gap-2 sm:grid-cols-[1fr_1fr_auto] sm:items-end"
        onSubmit={(event) => {
          event.preventDefault()
          save(
            {
              codespacesRepository: repository,
              codespacesMachine: machine,
              codespacesDevcontainerPath: devcontainer,
              githubClientId: clientId,
            },
            'Codespaces settings saved.',
          )
        }}
      >
        <label className="block">
          <span className={fieldLabel}>Runtime repository</span>
          <Input
            value={repository}
            onChange={(event) => setRepository(event.target.value)}
            aria-label="Codespaces repository"
            className="mt-1.5 font-mono text-sm"
          />
        </label>
        <label className="block">
          <span className={fieldLabel}>Default machine</span>
          <Input
            value={machine}
            onChange={(event) => setMachine(event.target.value)}
            aria-label="Codespaces machine"
            className="mt-1.5 font-mono text-sm"
          />
        </label>
        <span />
        <label className="block">
          <span className={fieldLabel}>Dev container</span>
          <Input
            value={devcontainer}
            onChange={(event) => setDevcontainer(event.target.value)}
            aria-label="Dev container path"
            className="mt-1.5 font-mono text-sm"
          />
        </label>
        <label className="block">
          <span className={fieldLabel}>OAuth app client id (for Sign in)</span>
          <Input
            value={clientId}
            onChange={(event) => setClientId(event.target.value)}
            placeholder="Iv1.…"
            aria-label="GitHub OAuth client id"
            className="mt-1.5 font-mono text-sm"
          />
        </label>
        <Button type="submit" variant="secondary" size="sm" disabled={saving}>
          <Save className="size-4" /> Save
        </Button>
      </form>
      <p className="text-xs leading-relaxed text-graphite">
        The repository's dev container starts Jupyter on port 8888; NZAP Engine reaches it through
        GitHub's private port forwarding. Codespaces bill your GitHub account.
      </p>
    </Section>
  )
}

function CodesandboxSection({
  status,
  view,
  save,
  saving,
}: {
  status: ProviderStatus | undefined
  view: SettingsView
  save: (patch: SettingsPatch, message?: string) => void
  saving: boolean
}) {
  const settings = view.settings
  const keyMutation = useCodesandboxKey()
  const disconnect = useCodesandboxDisconnect()
  const [key, setKey] = useState('')
  const [template, setTemplate] = useState(settings.codesandboxTemplate)
  const [tier, setTier] = useState(settings.codesandboxTier)
  const connected = Boolean(status?.connected)

  return (
    <Section title="CodeSandbox" status={status}>
      {connected ? (
        <Button
          variant="ghost"
          size="sm"
          disabled={disconnect.isPending}
          onClick={() =>
            disconnect.mutate(undefined, {
              onSuccess: () => toast.success('CodeSandbox disconnected.'),
              onError: (error) => toast.error(errorMessage(error, 'Could not disconnect.')),
            })
          }
        >
          <LogOut className="size-4" /> Remove API key
        </Button>
      ) : (
        <form
          className="flex flex-wrap gap-2"
          onSubmit={(event) => {
            event.preventDefault()
            keyMutation.mutate(key, {
              onSuccess: () => {
                setKey('')
                toast.success('CodeSandbox connected.')
              },
              onError: (error) =>
                toast.error(errorMessage(error, 'CodeSandbox did not accept that key.')),
            })
          }}
        >
          <Input
            type="password"
            value={key}
            onChange={(event) => setKey(event.target.value)}
            placeholder="csb_…"
            aria-label="CodeSandbox API key"
            className="min-w-0 flex-1 font-mono text-sm"
          />
          <Button
            type="submit"
            variant="secondary"
            size="sm"
            disabled={!key.trim() || keyMutation.isPending}
          >
            <KeyRound className="size-4" /> Save key
          </Button>
        </form>
      )}
      <form
        className="grid gap-2 sm:grid-cols-[1fr_1fr_auto] sm:items-end"
        onSubmit={(event) => {
          event.preventDefault()
          save(
            { codesandboxTemplate: template, codesandboxTier: tier },
            'CodeSandbox settings saved.',
          )
        }}
      >
        <label className="block">
          <span className={fieldLabel}>Template</span>
          <Input
            value={template}
            onChange={(event) => setTemplate(event.target.value)}
            aria-label="CodeSandbox template"
            className="mt-1.5 font-mono text-sm"
          />
        </label>
        <label className="block">
          <span className={fieldLabel}>Default VM tier</span>
          <select
            value={tier}
            onChange={(event) => setTier(event.target.value)}
            aria-label="CodeSandbox VM tier"
            className="mt-1.5 h-11 w-full rounded-2xl border border-ink bg-transparent px-3 text-sm"
          >
            {TIERS.map((name) => (
              <option key={name} value={name}>
                {name}
              </option>
            ))}
          </select>
        </label>
        <Button type="submit" variant="secondary" size="sm" disabled={saving}>
          <Save className="size-4" /> Save
        </Button>
      </form>
      <p className="text-xs leading-relaxed text-graphite">
        Create an API key at codesandbox.io/t/api with VM and sandbox permissions. Each new VM
        installs and starts Jupyter, then hibernates when idle and wakes on the next use.
      </p>
    </Section>
  )
}
