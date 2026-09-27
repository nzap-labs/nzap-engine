import { useQuery } from '@tanstack/react-query'
import type { ReactNode } from 'react'
import { Clock, Cpu, Gauge, ShieldCheck, Wallet } from 'lucide-react'
import { colabQuotaQuery, colabSessionsQuery, colabStatusQuery } from '@/api/colab'
import { Avatar } from '@/components/avatar'
import { ExternalLink } from '@/components/external-link'
import { ConnectionCard } from '@/features/colab/connection-card'
import { PageHeader } from '@/features/shell/page-header'
import { formatDateTime } from '@/lib/format'

const TIER_LABELS: Record<string, string> = {
  NONE: 'Free',
  PRO: 'Colab Pro',
  PRO_PLUS: 'Colab Pro+',
}

/**
 * Who is connected and what their Colab account can do — the desktop
 * replacement for hosted NZAP's profile, account and credits pages.
 */
export function AccountPage() {
  const { data: status } = useQuery(colabStatusQuery)
  const { data: quota, error: quotaError } = useQuery({
    ...colabQuotaQuery,
    enabled: Boolean(status?.connected),
  })
  const { data: sessions } = useQuery(colabSessionsQuery)
  const user = status?.user

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <PageHeader title="Account" />
      <div className="scrollbar-thin min-h-0 flex-1 overflow-y-auto px-4 pb-10 md:px-6">
        <div className="mx-auto w-full max-w-3xl space-y-6">
          {user && (
            <section className="flex items-center gap-4 rounded-[24px] border border-ink bg-paper p-6">
              <Avatar
                src={user.picture || undefined}
                name={user.name || user.email}
                className="size-14 text-lg"
              />
              <div className="min-w-0">
                <p className="truncate text-xl font-medium tracking-tight">
                  {user.name || user.email}
                </p>
                <p className="truncate text-sm text-graphite">{user.email}</p>
              </div>
            </section>
          )}

          <ConnectionCard />

          {status?.connected && (
            <section
              aria-label="Colab compute"
              className="rounded-[24px] border border-ink bg-paper p-6"
            >
              <p className="font-medium">Colab compute</p>
              <p className="mt-1 text-sm text-graphite">
                Straight from Google, refreshed every minute — the same numbers the Colab VS Code
                extension shows.
              </p>
              {quota ? (
                <>
                  <dl className="mt-5 grid gap-3 sm:grid-cols-2">
                    <Fact
                      icon={<ShieldCheck />}
                      label="Plan"
                      value={TIER_LABELS[quota.tier] ?? quota.tier}
                    />
                    <Fact
                      icon={<Wallet />}
                      label="Paid compute units"
                      value={quota.paidComputeUnits.toFixed(2)}
                    />
                    <Fact
                      icon={<Gauge />}
                      label="Burn rate"
                      value={`${quota.consumptionRateHourly.toFixed(2)} units / hour`}
                    />
                    <Fact
                      icon={<Clock />}
                      label="Time left at this rate"
                      value={
                        quota.minutesRemaining === null
                          ? 'nothing is running'
                          : `${Math.floor(quota.minutesRemaining / 60)}h ${quota.minutesRemaining % 60}m`
                      }
                    />
                    {quota.freeCcuRemaining !== null && (
                      <Fact
                        icon={<Wallet />}
                        label="Free units left"
                        value={quota.freeCcuRemaining.toFixed(2)}
                      />
                    )}
                    {quota.nextFreeRefillAt !== null && (
                      <Fact
                        icon={<Clock />}
                        label="Free units refill"
                        value={formatDateTime(
                          new Date(quota.nextFreeRefillAt * 1000).toISOString(),
                        )}
                      />
                    )}
                    <Fact
                      icon={<Cpu />}
                      label="Runtimes in NZAP Engine"
                      value={String(sessions?.sessions.length ?? 0)}
                    />
                  </dl>
                  {quota.eligibleAccelerators.length > 0 && (
                    <p className="mt-4 text-sm text-graphite">
                      Available accelerators:{' '}
                      <span className="font-medium text-ink">
                        {quota.eligibleAccelerators.join(', ')}
                      </span>
                    </p>
                  )}
                  <p className="mt-4 whitespace-pre-line rounded-2xl border border-line bg-paper-soft p-4 text-xs leading-relaxed text-graphite">
                    {quota.tooltip}
                  </p>
                  <ExternalLink
                    href="https://colab.research.google.com/signup"
                    className="mt-4 inline-flex h-9 items-center rounded-3xl border border-ink px-4 text-sm font-medium transition-colors hover:bg-paper-soft"
                  >
                    {quota.signupAction}
                  </ExternalLink>
                </>
              ) : quotaError ? (
                <p className="mt-4 text-sm text-graphite">
                  Colab did not return compute details: {quotaError.message}
                </p>
              ) : (
                <div className="mt-4 h-24 animate-pulse rounded-2xl bg-paper-soft" />
              )}
            </section>
          )}

          <section aria-label="Privacy" className="rounded-[24px] border border-line bg-paper p-6">
            <p className="font-medium">Privacy</p>
            <p className="mt-1 text-sm leading-relaxed text-graphite">
              NZAP Engine has no account and no server. Your Google token lives on this computer
              {status?.storage === 'keychain' ? ' in the system keychain' : ''}, notebooks and
              history stay in the app&apos;s data folder, and nothing is sent anywhere except to
              Google and to GitHub for the public notebook collection.
            </p>
            <ExternalLink
              href="https://myaccount.google.com/permissions"
              className="mt-3 inline-block text-sm font-medium underline underline-offset-4"
            >
              Review access in your Google account
            </ExternalLink>
          </section>
        </div>
      </div>
    </div>
  )
}

function Fact({ icon, label, value }: { icon: ReactNode; label: string; value: string }) {
  return (
    <div className="rounded-2xl border border-line bg-paper-soft p-3">
      <dt className="flex items-center gap-1.5 text-xs text-graphite [&_svg]:size-3.5">
        {icon}
        {label}
      </dt>
      <dd className="mt-1 truncate text-sm font-medium">{value}</dd>
    </div>
  )
}
