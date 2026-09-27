import { useQuery } from '@tanstack/react-query'
import { useState } from 'react'
import type { ReactNode } from 'react'
import {
  CheckCircle2,
  KeyRound,
  Link2,
  Loader2,
  LogOut,
  RefreshCw,
  ShieldCheck,
} from 'lucide-react'
import { toast } from 'sonner'
import {
  cancelConnect,
  colabQuotaQuery,
  colabStatusQuery,
  useConnectColab,
  useDisconnectColab,
  useRemoteConnect,
} from '@/api/colab'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { cn } from '@/lib/cn'
import { errorMessage } from '@/lib/ipc'

const TIER_LABELS: Record<string, string> = {
  NONE: 'Free',
  PRO: 'Colab Pro',
  PRO_PLUS: 'Colab Pro+',
}

/**
 * "Google Auth" card: the connection of the user's Google account for Colab.
 * The dot is green only when the engine has verified the token against Colab
 * itself, not merely because a token is stored.
 */
export function ConnectionCard() {
  const { data: status, isPending } = useQuery(colabStatusQuery)
  const connect = useConnectColab()
  const disconnect = useDisconnectColab()
  const [confirming, setConfirming] = useState(false)

  const connected = Boolean(status?.connected)

  function startConnect() {
    connect.mutate(status?.email ?? undefined, {
      onSuccess: (user) => toast.success(`Google connected as ${user.email}.`),
      onError: (error) => {
        if (error instanceof Error && 'code' in error && error.code === 'cancelled') return
        toast.error(errorMessage(error, 'Could not connect Google.'))
      },
    })
  }

  return (
    <section
      aria-label="Google Auth"
      className="rounded-[24px] border border-ink bg-paper p-6 md:p-8"
    >
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="flex items-center gap-2.5">
            <span
              aria-hidden
              className={cn(
                'size-2.5 shrink-0 rounded-full',
                connected ? 'bg-mint' : 'bg-graphite/40',
              )}
            />
            <p className="font-medium">Google Auth</p>
            <span className="rounded-full border border-ink px-2.5 py-0.5 text-[11px] font-medium uppercase tracking-wide">
              {isPending ? 'checking…' : connected ? 'connected' : 'not connected'}
            </span>
          </div>
          <p className="mt-2 max-w-xl text-sm leading-relaxed text-graphite">
            {connected
              ? `Connected as ${status?.email ?? 'your Google account'}. NZAP Engine uses this account to allocate Colab runtimes; the token stays on this computer${status?.storage === 'keychain' ? ', in your system keychain' : ''} and is refreshed automatically.`
              : connect.isPending
                ? 'Finish signing in in your browser — this window updates as soon as Google redirects back.'
                : 'Connect a Google account to allocate Colab runtimes. NZAP Engine asks for the same Colab and Drive permissions the Colab notebook itself uses.'}
          </p>
          {connected && status?.warning && (
            <p className="mt-2 text-xs text-graphite">{status.warning}</p>
          )}
          {!connected && status?.reason === 'revoked' && (
            <p className="mt-2 text-xs text-graphite">
              Google no longer accepts the stored connection
              {status.email ? ` for ${status.email}` : ''}. Connect again to keep using Colab
              runtimes.
            </p>
          )}
          {connected && status?.storage === 'file' && (
            <p className="mt-2 text-xs text-coral">
              No system keychain was available, so the token is stored in a file only your user
              account can read.
            </p>
          )}
        </div>

        <div className="flex shrink-0 flex-wrap items-center gap-2">
          {connected ? (
            <Dialog open={confirming} onOpenChange={setConfirming}>
              <DialogTrigger asChild>
                <Button variant="secondary" size="sm">
                  <LogOut className="size-4" /> Disconnect
                </Button>
              </DialogTrigger>
              <DialogContent>
                <DialogTitle>Disconnect Google Auth?</DialogTitle>
                <DialogDescription>
                  NZAP Engine releases every Colab runtime it is running, revokes its access to your
                  Google account and forgets the stored token. Notebooks and files on Google Drive
                  are not touched.
                </DialogDescription>
                <div className="mt-5 flex justify-end gap-2">
                  <DialogClose asChild>
                    <Button variant="ghost" size="sm">
                      Cancel
                    </Button>
                  </DialogClose>
                  <Button
                    variant="danger"
                    size="sm"
                    disabled={disconnect.isPending}
                    onClick={() =>
                      disconnect.mutate(undefined, {
                        onSuccess: () => {
                          setConfirming(false)
                          toast.success('Google Auth disconnected.')
                        },
                        onError: (error) =>
                          toast.error(errorMessage(error, 'Could not disconnect.')),
                      })
                    }
                  >
                    {disconnect.isPending ? 'Disconnecting…' : 'Disconnect'}
                  </Button>
                </div>
              </DialogContent>
            </Dialog>
          ) : connect.isPending ? (
            <>
              <Button size="sm" disabled>
                <Loader2 className="size-4 animate-spin" /> Waiting for Google…
              </Button>
              <Button variant="ghost" size="sm" onClick={() => void cancelConnect()}>
                Cancel
              </Button>
            </>
          ) : (
            <>
              <Button size="sm" onClick={startConnect}>
                <Link2 className="size-4" /> Connect Google
              </Button>
              <RemoteConnectDialog />
            </>
          )}
        </div>
      </div>

      {connected && <AccountSummary storage={status?.storage} />}
    </section>
  )
}

/**
 * The copy/paste fallback (google-colab-cli's remote flow) for when the
 * browser cannot reach this computer's loopback address.
 */
function RemoteConnectDialog() {
  const { begin, complete } = useRemoteConnect()
  const [open, setOpen] = useState(false)
  const [code, setCode] = useState('')

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        setOpen(next)
        if (!next) setCode('')
      }}
    >
      <DialogTrigger asChild>
        <Button variant="ghost" size="sm">
          <KeyRound className="size-4" /> Use a code
        </Button>
      </DialogTrigger>
      <DialogContent>
        <DialogTitle>Connect with a code</DialogTitle>
        <DialogDescription>
          Use this if the browser sign-in cannot finish (for example on a remote desktop). Google
          shows a code after you approve access; paste it here.
        </DialogDescription>
        <div className="mt-4 space-y-3">
          <Button
            variant="secondary"
            size="sm"
            disabled={begin.isPending}
            onClick={() =>
              begin.mutate(undefined, {
                onError: (error) => toast.error(errorMessage(error, 'Could not open Google.')),
              })
            }
          >
            1. Open Google sign-in
          </Button>
          <form
            className="flex gap-2"
            onSubmit={(event) => {
              event.preventDefault()
              complete.mutate(code, {
                onSuccess: (user) => {
                  setOpen(false)
                  setCode('')
                  toast.success(`Google connected as ${user.email}.`)
                },
                onError: (error) => toast.error(errorMessage(error, 'That code did not work.')),
              })
            }}
          >
            <Input
              value={code}
              onChange={(event) => setCode(event.target.value)}
              placeholder="2. Paste the code"
              aria-label="Authorization code"
              autoComplete="off"
              spellCheck={false}
            />
            <Button type="submit" disabled={!code.trim() || complete.isPending || !begin.isSuccess}>
              {complete.isPending ? <Loader2 className="size-4 animate-spin" /> : 'Connect'}
            </Button>
          </form>
        </div>
      </DialogContent>
    </Dialog>
  )
}

function AccountSummary({ storage }: { storage?: string }) {
  const { data: quota } = useQuery(colabQuotaQuery)
  if (!quota) return null

  const units =
    quota.paidComputeUnits > 0
      ? `${quota.paidComputeUnits.toFixed(1)} paid`
      : quota.freeCcuRemaining !== null
        ? `${quota.freeCcuRemaining.toFixed(1)} free`
        : '—'

  return (
    <dl className="mt-5 grid gap-3 border-t border-line pt-5 sm:grid-cols-3">
      <Fact
        icon={<ShieldCheck className="size-4" />}
        label="Colab plan"
        value={TIER_LABELS[quota.tier] ?? quota.tier}
      />
      <Fact icon={<RefreshCw className="size-4" />} label="Compute units" value={units} />
      <Fact
        icon={<CheckCircle2 className="size-4" />}
        label="Token"
        value={storage === 'keychain' ? 'in system keychain' : 'verified with Colab'}
      />
    </dl>
  )
}

function Fact({ icon, label, value }: { icon: ReactNode; label: string; value: string }) {
  return (
    <div className="rounded-2xl border border-line bg-paper-soft p-3">
      <dt className="flex items-center gap-1.5 text-xs text-graphite">
        <span className="[&_svg]:size-3.5">{icon}</span>
        {label}
      </dt>
      <dd className="mt-1 truncate text-sm font-medium">{value}</dd>
    </div>
  )
}
