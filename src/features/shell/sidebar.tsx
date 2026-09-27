import type { ReactNode } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Link, useRouterState } from '@tanstack/react-router'
import { toast } from 'sonner'
import {
  Cpu,
  Gauge,
  PanelLeftClose,
  Plus,
  Presentation,
  Settings,
  SlidersHorizontal,
  Smile,
  SquarePen,
  UserRound,
} from 'lucide-react'
import { colabQuotaQuery, colabStatusQuery } from '@/api/colab'
import { cn } from '@/lib/cn'
import { Avatar } from '@/components/avatar'
import { LogoWordmark } from '@/components/logo'
import { ThemeToggle } from '@/components/theme-toggle'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import type { ColabStatus } from '@/types/colab'

const HISTORY_MONTH = new Date().toLocaleString('en-US', { month: 'long' })

function SidebarItem({
  icon,
  label,
  active = false,
  onClick,
  to,
}: {
  icon: ReactNode
  label: string
  active?: boolean
  onClick?: () => void
  to?: string
}) {
  const classes = cn(
    'relative flex w-full cursor-pointer items-center gap-3 rounded-lg px-3 py-2 text-sm font-medium transition-colors',
    active ? 'text-ink' : 'text-graphite hover:bg-paper hover:text-ink',
  )
  const inner = (
    <>
      {active && (
        <span
          aria-hidden
          className="absolute left-0 top-1/2 h-4 w-1 -translate-y-1/2 rounded-r-full bg-sunshine"
        />
      )}
      <span className="[&_svg]:size-4">{icon}</span>
      {label}
    </>
  )

  if (to) {
    return (
      <Link to={to} className={classes} aria-current={active ? 'page' : undefined}>
        {inner}
      </Link>
    )
  }
  return (
    <button type="button" onClick={onClick} className={classes}>
      {inner}
    </button>
  )
}

/** Compute units at a glance (replaces hosted NZAP's credits card). */
function ComputeCard({ enabled }: { enabled: boolean }) {
  const { data: quota } = useQuery({ ...colabQuotaQuery, enabled })
  if (!enabled || !quota) return null
  const balance =
    quota.paidComputeUnits > 0
      ? `${quota.paidComputeUnits.toFixed(1)} compute units`
      : quota.freeCcuRemaining !== null
        ? `${quota.freeCcuRemaining.toFixed(1)} free units`
        : 'Free tier'
  return (
    <Link
      to="/account"
      className="block rounded-2xl border border-ink bg-paper p-4 transition-colors hover:bg-paper-soft"
    >
      <p className="flex items-center gap-2 text-sm font-medium">
        <Gauge className="size-4" /> Colab compute
      </p>
      <p className="mt-1 text-xs leading-relaxed text-graphite">
        {balance} · burning {quota.statusText}
      </p>
    </Link>
  )
}

function IdentityMenu({ status }: { status: ColabStatus }) {
  const user = status.user
  const label = user?.name || user?.email || 'Not connected'
  return (
    <DropdownMenu>
      <DropdownMenuTrigger className="flex w-full cursor-pointer items-center gap-3 rounded-2xl border border-transparent p-2 text-left transition-colors hover:border-line">
        {user ? (
          <Avatar src={user.picture || undefined} name={label} className="size-8" />
        ) : (
          <span className="grid size-8 shrink-0 place-items-center rounded-full border border-line text-graphite">
            <UserRound className="size-4" />
          </span>
        )}
        <span className="min-w-0 flex-1">
          <span className="block truncate text-sm font-medium">{label}</span>
          <span className="flex items-center gap-1.5 text-xs text-graphite">
            <span
              aria-hidden
              className={cn(
                'size-1.5 rounded-full',
                status.connected ? 'bg-mint' : 'bg-graphite/40',
              )}
            />
            {status.connected ? 'Google connected' : 'Google not connected'}
          </span>
        </span>
        <Settings className="size-4 shrink-0 text-graphite" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" side="top" className="min-w-[240px]">
        <DropdownMenuLabel className="normal-case">
          <span className="block truncate text-sm font-medium text-ink">
            {user?.email ?? 'No Google account'}
          </span>
          <span className="mt-0.5 block">Self-hosted · open source</span>
        </DropdownMenuLabel>
        <DropdownMenuSeparator />
        <DropdownMenuItem asChild>
          <Link to="/account">
            <UserRound className="size-4" /> Account
          </Link>
        </DropdownMenuItem>
        <DropdownMenuItem asChild>
          <Link to="/settings">
            <SlidersHorizontal className="size-4" /> Settings
          </Link>
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

export function Sidebar({ onClose }: { onClose: () => void }) {
  const { data: status } = useQuery(colabStatusQuery)
  const pathname = useRouterState({ select: (state) => state.location.pathname })

  return (
    <aside className="flex h-full w-[300px] shrink-0 flex-col overflow-y-auto border-r border-line bg-paper scrollbar-thin">
      <div className="flex items-center justify-between p-4">
        <LogoWordmark />
        <div className="flex items-center gap-1">
          <ThemeToggle />
          <Tooltip>
            <TooltipTrigger asChild>
              <button
                type="button"
                aria-label="Close sidebar"
                onClick={onClose}
                className="cursor-pointer rounded-lg p-2 text-graphite transition-colors hover:bg-paper-soft hover:text-ink"
              >
                <PanelLeftClose className="size-4" />
              </button>
            </TooltipTrigger>
            <TooltipContent>Close sidebar</TooltipContent>
          </Tooltip>
        </div>
      </div>

      <nav
        aria-label="Workspace modes"
        className="mx-4 rounded-2xl border border-ink bg-paper-soft p-1.5"
      >
        <SidebarItem icon={<SquarePen />} label="Chat" to="/chat" active={pathname === '/chat'} />
        <SidebarItem
          icon={<Smile />}
          label="Agent"
          onClick={() => toast.info('Agent mode is on the roadmap.')}
        />
      </nav>

      <div className="mt-4 space-y-1 px-4">
        <SidebarItem
          icon={<Plus />}
          label="New Chat"
          onClick={() => toast.info('Chat arrives in a later release.')}
        />
        <SidebarItem icon={<Cpu />} label="Colab" to="/colab" active={pathname === '/colab'} />
        <SidebarItem
          icon={<Presentation />}
          label="AI Slides"
          onClick={() => toast.info('AI Slides is on the roadmap.')}
        />
      </div>

      <div className="mt-6 px-6 text-xs font-medium uppercase tracking-[0.14em] text-graphite">
        {HISTORY_MONTH}
      </div>
      <div className="px-6 pt-2">
        <p className="text-xs leading-relaxed text-graphite">
          No conversations yet — your chats will appear here.
        </p>
      </div>

      <div className="mt-auto space-y-3 p-4">
        <ComputeCard enabled={Boolean(status?.connected)} />
        {status && <IdentityMenu status={status} />}
      </div>
    </aside>
  )
}
