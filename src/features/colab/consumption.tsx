import { useEffect, useRef } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Gauge } from 'lucide-react'
import { toast } from 'sonner'
import { colabQuotaQuery } from '@/api/colab'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { cn } from '@/lib/cn'
import { openExternal } from '@/lib/ipc'
import type { ColabQuota } from '@/types/colab'

const SIGNUP_URL = 'https://colab.research.google.com/signup'

/**
 * Header chip mirroring the VS Code extension's status-bar item: the hourly
 * compute-unit burn rate, with the extension's tooltip (tier, balance, and
 * how long free-tier runtimes may last) on hover.
 */
export function ConsumptionChip({ enabled }: { enabled: boolean }) {
  const { data: quota } = useQuery({ ...colabQuotaQuery, enabled })
  useLowBalanceNotifier(quota)
  if (!enabled || !quota) return null

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span
          tabIndex={0}
          className={cn(
            'flex items-center gap-1.5 rounded-full border px-3 py-1 font-mono text-xs',
            quota.severity === 'ok' ? 'border-ink' : 'border-coral text-coral',
          )}
        >
          <Gauge className="size-3.5" aria-hidden />
          {quota.statusText}
          {quota.minutesRemaining !== null && (
            <span className="text-graphite">· {formatMinutes(quota.minutesRemaining)} left</span>
          )}
        </span>
      </TooltipTrigger>
      <TooltipContent className="max-w-xs whitespace-pre-line font-normal leading-relaxed">
        {quota.tooltip}
      </TooltipContent>
    </Tooltip>
  )
}

/**
 * Port of `ConsumptionNotifier`: warn when less than 30 minutes of compute
 * remain, raise an error once it is depleted, and snooze each kind for ten
 * minutes after it fires so a one-minute poll does not spam the user.
 */
function useLowBalanceNotifier(quota: ColabQuota | undefined) {
  const snoozedUntil = useRef<{ low: number; depleted: number }>({ low: 0, depleted: 0 })

  useEffect(() => {
    if (!quota || quota.severity === 'ok') return
    const kind = quota.severity
    const now = Date.now()
    if (snoozedUntil.current[kind] > now) return
    snoozedUntil.current[kind] = now + quota.snoozeMinutes * 60_000

    const action = {
      label: quota.signupAction,
      onClick: () => void openExternal(SIGNUP_URL).catch(() => undefined),
    }
    if (kind === 'depleted') {
      toast.error('Colab Compute Units (CCU) depleted!', { action, duration: 15_000 })
    } else {
      toast.warning(
        `Low Colab Compute Units (CCU) balance! ${quota.minutesRemaining ?? 0} minutes left.`,
        { action, duration: 15_000 },
      )
    }
  }, [quota])
}

function formatMinutes(minutes: number): string {
  if (minutes <= 0) return '0m'
  const hours = Math.floor(minutes / 60)
  return hours > 0 ? `${hours}h${minutes % 60}m` : `${minutes}m`
}
